use std::panic::{self, AssertUnwindSafe};

use k256::ecdsa::signature::DigestSigner;
use k256::ecdsa::{Signature, SigningKey};
use orderbook::orderbook::{
    ExecutionMode, Order, OrderSide, OrderType, Orderbook, PairInfo, TokenPair,
};
use orderbook::smt_values::UserInfo;
use orderbook::{
    AddSessionKeyPrivateInput, CreateOrderPrivateInput, OrderbookAction,
    PermissionnedOrderbookAction, PermissionnedPrivateInput,
};
use sdk::guest;
use sdk::{BlobIndex, Calldata, ContractName, Identity, LaneId, TxContext, TxHash};
use sha3::{Digest, Sha3_256};

struct TestSigner {
    signing_key: SigningKey,
    public_key: Vec<u8>,
}

impl TestSigner {
    fn new(seed: u8) -> Self {
        let field_bytes = k256::FieldBytes::from([seed; 32]);
        let signing_key = SigningKey::from_bytes(&field_bytes).expect("signing key");
        let public_key = signing_key
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        Self {
            signing_key,
            public_key,
        }
    }

    fn sign(&self, msg: &str) -> Vec<u8> {
        let mut hasher = Sha3_256::new();
        hasher.update(msg.as_bytes());
        let signature: Signature = self.signing_key.sign_digest(hasher);
        signature.to_vec()
    }
}

fn register_user(orderbook: &mut Orderbook, user_info: &UserInfo) {
    orderbook
        .update_user_info_merkle_root(user_info)
        .expect("register user in SMT");
}

fn test_user(name: &str) -> UserInfo {
    let mut user = UserInfo::new(name.to_string(), name.as_bytes().to_vec());
    user.nonce = 1;
    user.session_keys = Vec::new();
    user
}

#[allow(clippy::too_many_arguments)]
fn run_action(
    light: &mut Orderbook,
    full: &mut Orderbook,
    user: &str,
    action: PermissionnedOrderbookAction,
    private_payload: Vec<u8>,
    secret: &[u8],
    contract_name: &ContractName,
    identity: &Identity,
    tx_hash: TxHash,
    tx_ctx: &TxContext,
) -> (Vec<orderbook::orderbook::OrderbookEvent>, Vec<u8>, Calldata) {
    let user_info_light = light
        .get_user_info(user)
        .expect("user registered in light state");
    let user_info_full = full
        .get_user_info(user)
        .expect("user registered in full state");

    let events = light
        .execute_permissionned_action(user_info_light.clone(), action.clone(), &private_payload)
        .expect("light execution");

    let metadata = full
        .derive_zkvm_commitment_metadata_from_events(&user_info_full, &events)
        .expect("derive metadata");

    let _ = full
        .execute_permissionned_action(user_info_full.clone(), action.clone(), &private_payload)
        .expect("full execution");

    let permissioned_private_input = PermissionnedPrivateInput {
        secret: secret.to_vec(),
        user_info: user_info_full,
        private_input: private_payload,
    };

    let calldata = Calldata {
        identity: identity.clone(),
        tx_hash,
        blobs: vec![
            OrderbookAction::PermissionnedOrderbookAction(action).as_blob(contract_name.clone())
        ]
        .into(),
        tx_blob_count: 1,
        index: BlobIndex(0),
        tx_ctx: Some(tx_ctx.clone()),
        private_input: borsh::to_vec(&permissioned_private_input).expect("serialize private input"),
    };

    (events, metadata, calldata)
}

#[test]
fn light_to_zkvm_flow_missing_user_info() {
    let secret = b"test-secret".to_vec();
    let lane_id = LaneId::default();

    let mut light = Orderbook::init(lane_id.clone(), ExecutionMode::Light, secret.clone()).unwrap();
    let mut full = Orderbook::init(lane_id.clone(), ExecutionMode::Full, secret.clone()).unwrap();

    let pair: TokenPair = ("HYLLAR".to_string(), "ORANJ".to_string());
    let pair_info = PairInfo {
        base_scale: 0,
        quote_scale: 0,
    };

    let user1 = test_user("user1");
    let user2 = test_user("user2");

    register_user(&mut light, &user1);
    register_user(&mut light, &user2);
    register_user(&mut full, &user1);
    register_user(&mut full, &user2);

    let signer1 = TestSigner::new(1);
    let signer2 = TestSigner::new(2);

    let contract_name = ContractName("orderbook".to_string());
    let identity = Identity::from("orderbook@orderbook");
    let tx_ctx = TxContext {
        lane_id,
        ..Default::default()
    };

    let mut tx_counter = 0usize;
    let mut next_tx_hash = || {
        let hash = TxHash::from(format!("tx-{tx_counter}"));
        tx_counter += 1;
        hash
    };

    // 1. Create pair via light, replicate on full, execute in zkvm
    let (events, metadata, calldata) = run_action(
        &mut light,
        &mut full,
        &user1.user,
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info.clone(),
        },
        Vec::new(),
        &secret,
        &contract_name,
        &identity,
        next_tx_hash(),
        &tx_ctx,
    );
    assert!(matches!(
        events.as_slice(),
        [orderbook::orderbook::OrderbookEvent::PairCreated { .. }]
    ));
    let calldatas = vec![calldata];
    guest::execute::<Orderbook>(&metadata, &calldatas);

    // 2. Add session key for user1
    let add_key_payload = borsh::to_vec(&AddSessionKeyPrivateInput {
        new_public_key: signer1.public_key.clone(),
    })
    .unwrap();
    let (_, metadata, calldata) = run_action(
        &mut light,
        &mut full,
        &user1.user,
        PermissionnedOrderbookAction::AddSessionKey,
        add_key_payload,
        &secret,
        &contract_name,
        &identity,
        next_tx_hash(),
        &tx_ctx,
    );
    let calldatas = vec![calldata];
    guest::execute::<Orderbook>(&metadata, &calldatas);

    // 3. Deposit ORANJ for user1
    let (_, metadata, calldata) = run_action(
        &mut light,
        &mut full,
        &user1.user,
        PermissionnedOrderbookAction::Deposit {
            token: pair.1.clone(),
            amount: 100,
        },
        Vec::new(),
        &secret,
        &contract_name,
        &identity,
        next_tx_hash(),
        &tx_ctx,
    );
    let calldatas = vec![calldata];
    guest::execute::<Orderbook>(&metadata, &calldatas);

    // 4. Create bid order for user1
    let maker_order = Order {
        order_id: "id1".to_string(),
        order_type: OrderType::Limit,
        order_side: OrderSide::Bid,
        price: Some(1),
        pair: pair.clone(),
        quantity: 2,
    };

    let maker_user_info = full
        .get_user_info(&user1.user)
        .expect("maker in full state");
    let maker_message = format!(
        "{}:{}:create_order:{}",
        maker_user_info.user, maker_user_info.nonce, maker_order.order_id
    );
    let maker_signature = signer1.sign(&maker_message);

    let maker_private_input = borsh::to_vec(&CreateOrderPrivateInput {
        signature: maker_signature,
        public_key: signer1.public_key.clone(),
    })
    .unwrap();

    let (_, metadata, calldata) = run_action(
        &mut light,
        &mut full,
        &user1.user,
        PermissionnedOrderbookAction::CreateOrder {
            order_id: maker_order.order_id.clone(),
            order_side: maker_order.order_side.clone(),
            order_type: maker_order.order_type.clone(),
            price: maker_order.price,
            pair: maker_order.pair.clone(),
            quantity: maker_order.quantity,
        },
        maker_private_input,
        &secret,
        &contract_name,
        &identity,
        next_tx_hash(),
        &tx_ctx,
    );
    let calldatas = vec![calldata];
    guest::execute::<Orderbook>(&metadata, &calldatas);

    // 5. Add session key for user2
    let add_key_payload = borsh::to_vec(&AddSessionKeyPrivateInput {
        new_public_key: signer2.public_key.clone(),
    })
    .unwrap();
    let (_, metadata, calldata) = run_action(
        &mut light,
        &mut full,
        &user2.user,
        PermissionnedOrderbookAction::AddSessionKey,
        add_key_payload,
        &secret,
        &contract_name,
        &identity,
        next_tx_hash(),
        &tx_ctx,
    );
    let calldatas = vec![calldata];
    guest::execute::<Orderbook>(&metadata, &calldatas);

    // 6. Deposit HYLLAR for user2
    let (_, metadata, calldata) = run_action(
        &mut light,
        &mut full,
        &user2.user,
        PermissionnedOrderbookAction::Deposit {
            token: pair.0.clone(),
            amount: 100,
        },
        Vec::new(),
        &secret,
        &contract_name,
        &identity,
        next_tx_hash(),
        &tx_ctx,
    );
    let calldatas = vec![calldata];
    guest::execute::<Orderbook>(&metadata, &calldatas);

    // 7. User2 creates ask order that should match maker order
    let taker_order = Order {
        order_id: "id2".to_string(),
        order_type: OrderType::Limit,
        order_side: OrderSide::Ask,
        price: Some(1),
        pair: pair.clone(),
        quantity: 2,
    };

    let taker_user_info = full
        .get_user_info(&user2.user)
        .expect("taker in full state");
    let taker_message = format!(
        "{}:{}:create_order:{}",
        taker_user_info.user, taker_user_info.nonce, taker_order.order_id
    );
    let taker_signature = signer2.sign(&taker_message);

    let taker_private_input = borsh::to_vec(&CreateOrderPrivateInput {
        signature: taker_signature,
        public_key: signer2.public_key.clone(),
    })
    .unwrap();

    let (_events, metadata, calldata) = run_action(
        &mut light,
        &mut full,
        &user2.user,
        PermissionnedOrderbookAction::CreateOrder {
            order_id: taker_order.order_id.clone(),
            order_side: taker_order.order_side.clone(),
            order_type: taker_order.order_type.clone(),
            price: taker_order.price,
            pair: taker_order.pair.clone(),
            quantity: taker_order.quantity,
        },
        taker_private_input,
        &secret,
        &contract_name,
        &identity,
        next_tx_hash(),
        &tx_ctx,
    );

    let calldatas = vec![calldata];
    // Deserialize zkvm state to inspect available witnesses.
    let zkvm_orderbook: Orderbook = borsh::from_slice(&metadata).expect("decode zkvm state");
    if let orderbook::orderbook::ExecutionState::ZkVm(state) = &zkvm_orderbook.execution_state {
        assert!(
            !state
                .users_info
                .value
                .iter()
                .any(|info| info.user == user1.user),
            "maker user unexpectedly present in zkvm witness"
        );
    } else {
        panic!("expected zkvm execution state");
    }

    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        guest::execute::<Orderbook>(&metadata, &calldatas);
    }));

    assert!(result.is_err(), "guest execution should panic");
    let panic_payload = result.err().unwrap();
    let panic_message = if let Some(msg) = panic_payload.downcast_ref::<String>() {
        msg.clone()
    } else if let Some(msg) = panic_payload.downcast_ref::<&str>() {
        msg.to_string()
    } else {
        format!("panic payload: {panic_payload:?}",)
    };

    assert!(
        panic_message.contains("Missing user info"),
        "unexpected panic message: {panic_message}"
    );
}
