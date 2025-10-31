#![cfg(test)]

use std::collections::{HashMap, HashSet};

use borsh::BorshDeserialize;
use k256::ecdsa::signature::DigestSigner;
use k256::ecdsa::{Signature, SigningKey};
use sdk::{guest, BlockHeight, LaneId, StateCommitment, ZkContract};
use sdk::{tracing, ContractAction};
use sdk::{BlobIndex, Calldata, ContractName, Identity, TxContext, TxHash};
use sha3::{Digest, Sha3_256};

use crate::model::{
    AssetInfo, ExecuteState, Order, OrderSide, OrderType, OrderbookEvent, Pair, PairInfo, UserInfo,
};
use crate::transaction::{
    AddSessionKeyPrivateInput, CreateOrderPrivateInput, OrderbookAction,
    PermissionnedOrderbookAction, PermissionnedPrivateInput,
};
use crate::ORDERBOOK_ACCOUNT_IDENTITY;
use crate::{
    order_manager::OrderManager,
    zk::{FullState, ZkVmState, H256},
};

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

fn test_user(name: &str) -> UserInfo {
    UserInfo::new(name.to_string(), name.as_bytes().to_vec())
}

fn get_ctx() -> (ContractName, Identity, TxContext, LaneId, Vec<u8>) {
    let cn: ContractName = ContractName("orderbook".to_owned());
    let id: Identity = Identity::from(ORDERBOOK_ACCOUNT_IDENTITY);
    let lane_id = LaneId::default();
    let tx_ctx: TxContext = TxContext {
        lane_id: lane_id.clone(),
        ..Default::default()
    };
    let secret: Vec<u8> = b"test-secret".to_vec();
    (cn, id, tx_ctx, lane_id, secret)
}

#[allow(dead_code)]
#[derive(BorshDeserialize)]
struct OwnedCommitment {
    users_info_root: H256,
    balances_roots: HashMap<String, H256>,
    assets: HashMap<String, AssetInfo>,
    orders: OrderManager,
    hashed_secret: [u8; 32],
    lane_id: LaneId,
    last_block_number: BlockHeight,
}

fn run_action(
    light: &mut ExecuteState,
    full: &mut FullState,
    user: &str,
    action: PermissionnedOrderbookAction,
    private_payload: Vec<u8>,
) -> Vec<OrderbookEvent> {
    let action_repr = format!("{action:?}");
    let (cn, id, tx_ctx, _, secret) = get_ctx();

    let user_info = light
        .get_user_info(user)
        .unwrap_or_else(|_| test_user(user));

    let events = light
        .execute_permissionned_action(user_info.clone(), action.clone(), &private_payload)
        .expect("light execution");

    tracing::debug!("light events: {:?}", events);

    let commitment_metadata = full
        .derive_zkvm_commitment_metadata_from_events(&user_info, &events, &action)
        .expect("derive metadata");

    full.apply_events_and_update_roots(&user_info, events.clone())
        .expect("full execution deposit");

    let permissioned_private_input = PermissionnedPrivateInput {
        secret: secret.to_vec(),
        user_info: user_info.clone(),
        private_input: private_payload,
    };

    let calldata = Calldata {
        identity: id.clone(),
        blobs: vec![OrderbookAction::PermissionnedOrderbookAction(action, 0).as_blob(cn.clone())]
            .into(),
        tx_blob_count: 1,
        index: BlobIndex(0),
        tx_hash: TxHash::from("test-tx-hash"),
        tx_ctx: Some(tx_ctx.clone()),
        private_input: borsh::to_vec(&permissioned_private_input).expect("serialize private input"),
    };

    let res = guest::execute::<ZkVmState>(&commitment_metadata, &[calldata]);

    assert!(res.len() == 1, "expected one output");
    let hyli_output = &res[0];
    if !hyli_output.success {
        let metadata_state: ZkVmState =
            borsh::from_slice(&commitment_metadata).expect("decode zkvm metadata");
        panic!(
            "execution failed for action {action_repr}: {hyli_output:?}; known owners: {:?}; metadata owners: {:?}",
            full
                .state
                .order_manager
                .orders_owner
                .keys()
                .collect::<Vec<_>>(),
            metadata_state
                .order_manager
                .orders_owner
                .keys()
                .collect::<Vec<_>>()
        );
    }

    let full_commit = full.commit();
    assert_eq!(
        hyli_output.next_state, full_commit,
        "Full next state mismatch for action {action_repr}"
    );

    events
}

#[derive(Default, Clone, Copy)]
struct BalanceExpectation {
    base: i128,
    quote: i128,
}

#[derive(Clone, Copy)]
struct BalanceDelta<'a> {
    user: &'a str,
    base: i128,
    quote: i128,
}

#[derive(Clone)]
struct OrderSpec {
    id: &'static str,
    side: OrderSide,
    price: Option<u64>,
    quantity: u64,
}

fn delta<'a>(user: &'a str, base: i128, quote: i128) -> BalanceDelta<'a> {
    BalanceDelta { user, base, quote }
}

fn qty(amount: u64) -> i128 {
    amount as i128
}

fn notional(amount: u64, price: u64) -> i128 {
    i128::from(amount) * i128::from(price)
}
fn apply_balance_deltas<'a>(
    expected: &mut HashMap<&'a str, BalanceExpectation>,
    deltas: &[BalanceDelta<'a>],
) {
    for delta in deltas {
        let entry = expected.get_mut(delta.user).expect("balance entry");
        entry.base += delta.base;
        entry.quote += delta.quote;
    }
}

#[track_caller]
fn assert_stage<'a>(
    stage: &str,
    light: &ExecuteState,
    full: &FullState,
    expected: &HashMap<&'a str, BalanceExpectation>,
    users: &[&'a str],
    base_symbol: &str,
    quote_symbol: &str,
) {
    for &user in users {
        let expected_entry = expected.get(user).expect("expected balances");
        let expected_base: u64 = expected_entry.base.try_into().expect("base >= 0");
        let expected_quote: u64 = expected_entry.quote.try_into().expect("quote >= 0");

        let light_user = light.get_user_info(user).expect("light user info");
        let full_user = full.state.get_user_info(user).expect("full user info");

        let light_base = light.get_balance(&light_user, base_symbol);
        let full_base = full.state.get_balance(&full_user, base_symbol);
        let light_quote = light.get_balance(&light_user, quote_symbol);
        let full_quote = full.state.get_balance(&full_user, quote_symbol);

        assert_eq!(
                light_base.0, expected_base,
                "{stage}: user {user} base ({base_symbol}) balance mismatch for light (expected {expected_base}, got {light_base:?})"
            );
        assert_eq!(
                full_base.0, expected_base,
                "{stage}: user {user} base ({base_symbol}) balance mismatch for full (expected {expected_base}, got {full_base:?})"
            );
        assert_eq!(
                light_quote.0, expected_quote,
                "{stage}: user {user} quote ({quote_symbol}) balance mismatch for light (expected {expected_quote}, got {light_quote:?})"
            );
        assert_eq!(
                full_quote.0, expected_quote,
                "{stage}: user {user} quote ({quote_symbol}) balance mismatch for full (expected {expected_quote}, got {full_quote:?})"
            );
    }
}

fn signer_for<'a>(users: &[&'a str], signers: &'a [TestSigner], user: &str) -> &'a TestSigner {
    let index = users
        .iter()
        .position(|candidate| *candidate == user)
        .expect("signer index");
    &signers[index]
}

fn submit_signed_order<'a>(
    light: &mut ExecuteState,
    full: &mut FullState,
    users: &[&'a str],
    signers: &'a [TestSigner],
    user: &str,
    order: Order,
) {
    let signer = signer_for(users, signers, user);
    let user_info = full
        .state
        .get_user_info(user)
        .expect("user info for signature");
    let order_id = order.order_id.clone();
    let msg = format!("{}:{}:create_order:{}", user, user_info.nonce, order_id);
    let signature = signer.sign(&msg);
    let private_input = CreateOrderPrivateInput {
        signature,
        public_key: signer.public_key.clone(),
    };
    let private_payload = borsh::to_vec(&private_input).expect("serialize create order input");

    let _ = run_action(
        light,
        full,
        user,
        PermissionnedOrderbookAction::CreateOrder(order),
        private_payload,
    );
}

fn add_session_key<'a>(
    light: &mut ExecuteState,
    full: &mut FullState,
    users: &[&'a str],
    signers: &'a [TestSigner],
    user: &str,
) {
    let signer = signer_for(users, signers, user);
    let payload = borsh::to_vec(&AddSessionKeyPrivateInput {
        new_public_key: signer.public_key.clone(),
    })
    .expect("serialize add session key input");

    let _ = run_action(
        light,
        full,
        user,
        PermissionnedOrderbookAction::AddSessionKey,
        payload,
    );
}

fn deposit(
    light: &mut ExecuteState,
    full: &mut FullState,
    user: &str,
    symbol: &str,
    amount: u64,
) -> Vec<OrderbookEvent> {
    run_action(
        light,
        full,
        user,
        PermissionnedOrderbookAction::Deposit {
            symbol: symbol.to_string(),
            amount,
        },
        Vec::new(),
    )
}

fn execute_deposit_with_zk_checks(
    light: &mut ExecuteState,
    full: &mut FullState,
    ctx: &(ContractName, Identity, TxContext, LaneId, Vec<u8>),
    user: &str,
    symbol: &str,
    amount: u64,
) -> StateCommitment {
    let (contract_name, identity, tx_ctx, _lane_id, secret) = ctx;

    let action = PermissionnedOrderbookAction::Deposit {
        symbol: symbol.to_string(),
        amount,
    };

    let user_info_light = light
        .get_user_info(user)
        .unwrap_or_else(|_| test_user(user));

    let events_light = light
        .execute_permissionned_action(user_info_light.clone(), action.clone(), &[])
        .expect("light execution deposit");

    let initial_commitment = full.commit();

    let user_info_full = full
        .state
        .get_user_info(user)
        .expect("user info before deposit");

    let metadata = full
        .derive_zkvm_commitment_metadata_from_events(&user_info_full, &events_light, &action)
        .expect("derive zk metadata for deposit");

    let zk_state: ZkVmState =
        borsh::from_slice(&metadata).expect("decode zk state for deposit metadata");
    let zk_initial_commitment = ZkContract::commit(&zk_state);
    assert_eq!(
        zk_initial_commitment, initial_commitment,
        "Initial commitment mismatch between FullState and ZkVmState metadata"
    );

    full.apply_events_and_update_roots(&user_info_full, events_light)
        .expect("full execution deposit");

    let private_input = PermissionnedPrivateInput {
        secret: secret.clone(),
        user_info: user_info_full.clone(),
        private_input: Vec::new(),
    };

    let calldata = Calldata {
        identity: identity.clone(),
        blobs: vec![
            OrderbookAction::PermissionnedOrderbookAction(action.clone(), 0)
                .as_blob(contract_name.clone()),
        ]
        .into(),
        tx_blob_count: 1,
        index: BlobIndex(0),
        tx_hash: TxHash::from("test-tx-hash"),
        tx_ctx: Some(tx_ctx.clone()),
        private_input: borsh::to_vec(&private_input)
            .expect("serialize permissionned private input for deposit"),
    };

    let outputs = guest::execute::<ZkVmState>(&metadata, &[calldata]);
    assert_eq!(outputs.len(), 1, "expected single zkvm output");
    let hyli_output = &outputs[0];
    if !hyli_output.success {
        panic!("deposit execution failed: {hyli_output:?}");
    }

    let final_commitment = full.commit();
    assert_eq!(
        hyli_output.next_state, final_commitment,
        "Next state mismatch between ZkVm output and FullState after deposit"
    );

    final_commitment
}

fn decode_commitment(commitment: &StateCommitment) -> OwnedCommitment {
    borsh::from_slice(&commitment.0).expect("decode state commitment")
}

fn execute_add_session_key_with_zk_checks(
    light: &mut ExecuteState,
    full: &mut FullState,
    ctx: &(ContractName, Identity, TxContext, LaneId, Vec<u8>),
    user: &str,
    new_key: Vec<u8>,
) -> StateCommitment {
    let (contract_name, identity, tx_ctx, _lane_id, secret) = ctx;

    let action = PermissionnedOrderbookAction::AddSessionKey;

    let user_info_light = light
        .get_user_info(user)
        .unwrap_or_else(|_| test_user(user));

    let private_payload = borsh::to_vec(&AddSessionKeyPrivateInput {
        new_public_key: new_key.clone(),
    })
    .expect("serialize add session key input");

    let events_light = light
        .execute_permissionned_action(user_info_light.clone(), action.clone(), &private_payload)
        .expect("light execution add session key");

    let initial_commitment = full.commit();

    let user_info_full = full
        .state
        .get_user_info(user)
        .expect("user info before session key addition");

    let metadata = full
        .derive_zkvm_commitment_metadata_from_events(&user_info_full, &events_light, &action)
        .expect("derive zk metadata for add session key");

    let zk_state: ZkVmState =
        borsh::from_slice(&metadata).expect("decode zk state for add session key metadata");
    let zk_initial_commitment = ZkContract::commit(&zk_state);
    assert_eq!(
        zk_initial_commitment, initial_commitment,
        "Initial commitment mismatch between FullState and ZkVmState metadata for add session key"
    );

    full.apply_events_and_update_roots(&user_info_full, events_light)
        .expect("full execution deposit");

    let private_input = PermissionnedPrivateInput {
        secret: secret.clone(),
        user_info: user_info_full.clone(),
        private_input: private_payload.clone(),
    };

    let calldata = Calldata {
        identity: identity.clone(),
        blobs: vec![
            OrderbookAction::PermissionnedOrderbookAction(action.clone(), 0)
                .as_blob(contract_name.clone()),
        ]
        .into(),
        tx_blob_count: 1,
        index: BlobIndex(0),
        tx_hash: TxHash::from("test-tx-hash"),
        tx_ctx: Some(tx_ctx.clone()),
        private_input: borsh::to_vec(&private_input)
            .expect("serialize permissionned private input for add session key"),
    };

    let outputs = guest::execute::<ZkVmState>(&metadata, &[calldata]);
    assert_eq!(outputs.len(), 1, "expected single zkvm output");
    let hyli_output = &outputs[0];
    if !hyli_output.success {
        panic!("add session key execution failed: {hyli_output:?}");
    }

    let final_commitment = full.commit();
    assert_eq!(
        hyli_output.next_state, final_commitment,
        "Next state mismatch between ZkVm output and FullState after add session key"
    );

    let parsed_next = decode_commitment(&hyli_output.next_state);
    assert!(
        parsed_next
            .users_info_root
            .as_ref()
            .iter()
            .any(|byte| *byte != 0),
        "users info root should be non-zero in zkvm state commitment after adding a session key"
    );

    final_commitment
}

#[test_log::test]
fn test_deposit_state_commitment() {
    let ctx = get_ctx();
    let lane_id = ctx.3.clone();
    let secret = ctx.4.clone();
    let mut light = ExecuteState::default();
    let mut full = FullState::from_data(
        &light,
        secret.clone(),
        lane_id.clone(),
        BlockHeight::default(),
    )
    .expect("building full state");

    let users = ["alice"];
    let signers = vec![TestSigner::new(1)];
    add_session_key(&mut light, &mut full, &users, &signers, users[0]);

    let base_symbol = "HYLLAR".to_string();
    let quote_symbol = "ORANJ".to_string();
    let pair = (base_symbol.clone(), quote_symbol.clone());
    let pair_info = PairInfo {
        base: AssetInfo::new(0, ContractName(base_symbol.clone())),
        quote: AssetInfo::new(0, ContractName(quote_symbol.clone())),
    };

    let _ = run_action(
        &mut light,
        &mut full,
        users[0],
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info,
        },
        Vec::new(),
    );

    let deposit_amount = 1_000_u64;
    let commitment = execute_deposit_with_zk_checks(
        &mut light,
        &mut full,
        &ctx,
        users[0],
        &base_symbol,
        deposit_amount,
    );
    let parsed = decode_commitment(&commitment);

    let base_root = parsed
        .balances_roots
        .get(&base_symbol)
        .unwrap_or_else(|| panic!("missing {base_symbol} root after deposit"));
    assert!(
        base_root.as_ref().iter().any(|byte| *byte != 0),
        "base symbol root should be non-zero after deposit"
    );
    assert!(
        !parsed.balances_roots.contains_key(&quote_symbol),
        "zero balances symbol should not appear in commitment"
    );
}

#[test_log::test]
fn test_multiple_deposits_state_commitment() {
    let ctx = get_ctx();
    let lane_id = ctx.3.clone();
    let secret = ctx.4.clone();
    let mut light = ExecuteState::default();
    let mut full = FullState::from_data(
        &light,
        secret.clone(),
        lane_id.clone(),
        BlockHeight::default(),
    )
    .expect("building full state");

    let users = ["alice"];
    let signers = vec![TestSigner::new(1)];
    add_session_key(&mut light, &mut full, &users, &signers, users[0]);

    let base_symbol = "HYLLAR".to_string();
    let quote_symbol = "ORANJ".to_string();
    let pair = (base_symbol.clone(), quote_symbol.clone());
    let pair_info = PairInfo {
        base: AssetInfo::new(0, ContractName(base_symbol.clone())),
        quote: AssetInfo::new(0, ContractName(quote_symbol.clone())),
    };

    let _ = run_action(
        &mut light,
        &mut full,
        users[0],
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info,
        },
        Vec::new(),
    );

    let first_amount = 1_000_u64;
    let first_commitment = execute_deposit_with_zk_checks(
        &mut light,
        &mut full,
        &ctx,
        users[0],
        &base_symbol,
        first_amount,
    );
    let first_parsed = decode_commitment(&first_commitment);

    let first_root = *first_parsed
        .balances_roots
        .get(&base_symbol)
        .unwrap_or_else(|| panic!("missing {base_symbol} root after first deposit"));
    assert!(
        first_root.as_ref().iter().any(|byte| *byte != 0),
        "base symbol root should be non-zero after first deposit"
    );
    assert!(
        !first_parsed.balances_roots.contains_key(&quote_symbol),
        "quote symbol should not appear after first deposit"
    );

    let second_amount = 2_500_u64;
    let second_commitment = execute_deposit_with_zk_checks(
        &mut light,
        &mut full,
        &ctx,
        users[0],
        &base_symbol,
        second_amount,
    );
    let second_parsed = decode_commitment(&second_commitment);

    let second_root = *second_parsed
        .balances_roots
        .get(&base_symbol)
        .unwrap_or_else(|| panic!("missing {base_symbol} root after second deposit"));
    assert!(
        second_root.as_ref().iter().any(|byte| *byte != 0),
        "base symbol root should remain non-zero after chained deposits"
    );
    assert_ne!(
        first_root, second_root,
        "root should change after cumulative deposit"
    );
    assert!(
        !second_parsed.balances_roots.contains_key(&quote_symbol),
        "quote symbol should not appear after chained deposits"
    );
}

#[test_log::test]
fn test_add_session_key_state_commitment() {
    let ctx = get_ctx();
    let lane_id = ctx.3.clone();
    let secret = ctx.4.clone();

    let mut light = ExecuteState::default();
    let mut full = FullState::from_data(
        &light,
        secret.clone(),
        lane_id.clone(),
        BlockHeight::default(),
    )
    .expect("building full state");

    let user = "alice";
    let signer = TestSigner::new(1);
    let base_user = test_user(user);

    light.users_info.insert(user.to_string(), base_user.clone());
    full.state
        .users_info
        .insert(user.to_string(), base_user.clone());
    full.users_info_mt
        .update_all(std::iter::once(base_user.clone()))
        .expect("prime users info tree");

    let final_commitment = execute_add_session_key_with_zk_checks(
        &mut light,
        &mut full,
        &ctx,
        user,
        signer.public_key.clone(),
    );

    let parsed = decode_commitment(&final_commitment);

    assert!(
        parsed
            .users_info_root
            .as_ref()
            .iter()
            .any(|byte| *byte != 0),
        "users info root should be non-zero after adding a session key"
    );

    assert!(parsed.balances_roots.is_empty(), "no balances expected");
    assert!(parsed.assets.is_empty(), "no assets expected");

    let session_user = full
        .state
        .get_user_info(user)
        .expect("user info after add session key");
    assert!(
        session_user.session_keys.contains(&signer.public_key),
        "session key should be registered in state"
    );
}

#[allow(clippy::too_many_arguments)]
#[track_caller]
fn execute_market_order<'a>(
    stage: &str,
    order: Order,
    user: &'a str,
    light: &mut ExecuteState,
    full: &mut FullState,
    users: &[&'a str],
    signers: &'a [TestSigner],
    expected: &mut HashMap<&'a str, BalanceExpectation>,
    base_symbol: &str,
    quote_symbol: &str,
    deltas: &[BalanceDelta<'a>],
) {
    submit_signed_order(light, full, users, signers, user, order);
    apply_balance_deltas(expected, deltas);
    assert_stage(
        stage,
        light,
        full,
        expected,
        users,
        base_symbol,
        quote_symbol,
    );
}

#[test_log::test]
fn test_complex_multi_user_orderbook() {
    let (_, _, _, lane_id, secret) = get_ctx();

    let mut light = ExecuteState::default();
    let mut full = FullState::from_data(
        &light,
        secret.clone(),
        lane_id.clone(),
        BlockHeight::default(),
    )
    .expect("building full state");

    let pair: Pair = ("HYLLAR".to_string(), "ORANJ".to_string());
    let base_symbol = pair.0.clone();
    let quote_symbol = pair.1.clone();
    let pair_info = PairInfo {
        base: AssetInfo::new(0, ContractName(base_symbol.clone())),
        quote: AssetInfo::new(0, ContractName(quote_symbol.clone())),
    };

    let users = ["alice", "bob", "charlie"];
    let (alice, bob, charlie) = (users[0], users[1], users[2]);
    let signers: Vec<TestSigner> = (0..users.len())
        .map(|idx| TestSigner::new((idx + 1) as u8))
        .collect();

    let mut expected_balances: HashMap<&str, BalanceExpectation> = users
        .iter()
        .map(|&user| (user, BalanceExpectation::default()))
        .collect();

    for &user in &users {
        add_session_key(&mut light, &mut full, &users, &signers, user);
    }

    let _ = run_action(
        &mut light,
        &mut full,
        alice,
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info.clone(),
        },
        Vec::new(),
    );

    let funded_amount = 10_000_u64;
    for &user in &users {
        let _ = deposit(&mut light, &mut full, user, &base_symbol, funded_amount);
        let _ = deposit(&mut light, &mut full, user, &quote_symbol, funded_amount);
        apply_balance_deltas(
            &mut expected_balances,
            &[
                delta(user, qty(funded_amount), 0),
                delta(user, 0, qty(funded_amount)),
            ],
        );
    }

    assert_stage(
        "after deposits",
        &light,
        &full,
        &expected_balances,
        &users,
        &base_symbol,
        &quote_symbol,
    );

    let limit_orders = vec![
        OrderSpec {
            id: "ask-lim1",
            side: OrderSide::Ask,
            price: Some(14),
            quantity: 30,
        },
        OrderSpec {
            id: "ask-lim2",
            side: OrderSide::Ask,
            price: Some(13),
            quantity: 35,
        },
        OrderSpec {
            id: "ask-lim3",
            side: OrderSide::Ask,
            price: Some(12),
            quantity: 25,
        },
        OrderSpec {
            id: "ask-lim4",
            side: OrderSide::Ask,
            price: Some(11),
            quantity: 20,
        },
        OrderSpec {
            id: "ask-lim5",
            side: OrderSide::Ask,
            price: Some(10),
            quantity: 30,
        },
        OrderSpec {
            id: "ask-lim6",
            side: OrderSide::Ask,
            price: Some(9),
            quantity: 40,
        },
        OrderSpec {
            id: "bid-lim1",
            side: OrderSide::Bid,
            price: Some(8),
            quantity: 20,
        },
        OrderSpec {
            id: "bid-lim2",
            side: OrderSide::Bid,
            price: Some(7),
            quantity: 15,
        },
        OrderSpec {
            id: "bid-lim3",
            side: OrderSide::Bid,
            price: Some(6),
            quantity: 25,
        },
        OrderSpec {
            id: "bid-lim4",
            side: OrderSide::Bid,
            price: Some(5),
            quantity: 20,
        },
        OrderSpec {
            id: "bid-lim5",
            side: OrderSide::Bid,
            price: Some(4),
            quantity: 15,
        },
        OrderSpec {
            id: "bid-lim6",
            side: OrderSide::Bid,
            price: Some(3),
            quantity: 10,
        },
    ];

    for (index, spec) in limit_orders.iter().enumerate() {
        let user = users[index % users.len()];
        let order = Order {
            order_id: spec.id.to_string(),
            order_side: spec.side.clone(),
            order_type: OrderType::Limit,
            price: spec.price,
            pair: pair.clone(),
            quantity: spec.quantity,
        };

        submit_signed_order(&mut light, &mut full, &users, &signers, user, order);

        match spec.side {
            OrderSide::Ask => apply_balance_deltas(
                &mut expected_balances,
                &[delta(user, -qty(spec.quantity), 0)],
            ),
            OrderSide::Bid => {
                let price = spec.price.expect("bid limit order should have price");
                apply_balance_deltas(
                    &mut expected_balances,
                    &[delta(user, 0, -notional(spec.quantity, price))],
                );
            }
        }
    }

    let buy_orders = light.order_manager.bid_orders.get(&pair).unwrap().clone();
    let sell_orders = light.order_manager.ask_orders.get(&pair).unwrap().clone();

    let all_order_ids: Vec<String> = buy_orders
        .iter()
        .chain(sell_orders.iter())
        .flat_map(|(_price, orders)| orders.iter().cloned())
        .collect();

    let limit_order_ids: HashSet<String> = limit_orders
        .iter()
        .map(|spec| spec.id.to_string())
        .collect();

    for order_id in &all_order_ids {
        assert!(
            limit_order_ids.contains(order_id),
            "Order id {order_id} from buy/sell orders not found in limit_orders"
        );
    }
    assert_eq!(
        all_order_ids.len(),
        limit_orders.len(),
        "Mismatch in number of orders in order book"
    );

    assert_stage(
        "after limit orders",
        &light,
        &full,
        &expected_balances,
        &users,
        &base_symbol,
        &quote_symbol,
    );

    execute_market_order(
        "after partially filling ask-lim6",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 20,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_symbol,
        &quote_symbol,
        &[
            delta(alice, qty(20), -notional(20, 9)),
            delta(charlie, 0, notional(20, 9)),
        ],
    );

    execute_market_order(
        "after clearing ask-lim6 and half of ask-lim5",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 35,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_symbol,
        &quote_symbol,
        &[
            delta(alice, qty(20), -notional(20, 9)),
            delta(alice, qty(15), -notional(15, 10)),
            delta(charlie, 0, notional(20, 9)),
            delta(bob, 0, notional(15, 10)),
        ],
    );

    execute_market_order(
        "after clearing ask-lim5",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 15,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_symbol,
        &quote_symbol,
        &[
            delta(alice, qty(15), -notional(15, 10)),
            delta(bob, 0, notional(15, 10)),
        ],
    );

    execute_market_order(
        "after self match on ask-lim4",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 10,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_symbol,
        &quote_symbol,
        &[
            delta(alice, qty(10), -notional(10, 11)),
            delta(alice, 0, notional(10, 11)),
        ],
    );

    execute_market_order(
        "after clearing remaining ask orders",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 100,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_symbol,
        &quote_symbol,
        &[
            delta(alice, qty(10), -notional(10, 11)),
            delta(alice, qty(25), -notional(25, 12)),
            delta(alice, qty(35), -notional(35, 13)),
            delta(alice, qty(30), -notional(30, 14)),
            delta(alice, 0, notional(10, 11)),
            delta(alice, 0, notional(30, 14)),
            delta(bob, 0, notional(35, 13)),
            delta(charlie, 0, notional(25, 12)),
        ],
    );

    let sell_orders = light.order_manager.ask_orders.get(&pair);
    assert!(sell_orders.is_none(), "all sell orders should be filled");

    execute_market_order(
        "after partially filling bid-lim1",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 10,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_symbol,
        &quote_symbol,
        &[
            delta(alice, -qty(10), notional(10, 8)),
            delta(alice, qty(10), 0),
        ],
    );

    execute_market_order(
        "after clearing bid-lim1 and half of bid-lim2",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 20,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_symbol,
        &quote_symbol,
        &[
            delta(alice, -qty(10), notional(10, 8)),
            delta(alice, -qty(10), notional(10, 7)),
            delta(alice, qty(10), 0),
            delta(bob, qty(10), 0),
        ],
    );

    execute_market_order(
        "after clearing bid-lim2",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 5,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_symbol,
        &quote_symbol,
        &[delta(alice, -qty(5), notional(5, 7)), delta(bob, qty(5), 0)],
    );

    execute_market_order(
        "after clearing remaining bid orders",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 70,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_symbol,
        &quote_symbol,
        &[
            delta(alice, -qty(25), notional(25, 6)),
            delta(alice, -qty(20), notional(20, 5)),
            delta(alice, -qty(15), notional(15, 4)),
            delta(alice, -qty(10), notional(10, 3)),
            delta(charlie, qty(25), 0),
            delta(alice, qty(20), 0),
            delta(bob, qty(15), 0),
            delta(charlie, qty(10), 0),
        ],
    );

    let sell_orders = light.order_manager.ask_orders.get(&pair);
    assert!(sell_orders.is_none(), "all sell orders should be filled");
    let buy_orders = light.order_manager.bid_orders.get(&pair);
    assert!(buy_orders.is_none(), "all buy orders should be filled");
}

#[test_log::test]
fn test_escape_cancels_orders_and_resets_balances() {
    let (_, _, _, lane_id, secret) = get_ctx();

    let mut light = ExecuteState::default();
    let mut full = FullState::from_data(
        &light,
        secret.clone(),
        lane_id.clone(),
        BlockHeight::default(),
    )
    .expect("building full state");

    let pair: Pair = ("HYLLAR".to_string(), "ORANJ".to_string());
    let pair_info = PairInfo {
        base: AssetInfo::new(0, ContractName(pair.0.clone())),
        quote: AssetInfo::new(0, ContractName(pair.1.clone())),
    };

    let users = ["alice"];
    let signers = vec![TestSigner::new(1)];
    let user = users[0];

    add_session_key(&mut light, &mut full, &users, &signers, user);

    let _ = run_action(
        &mut light,
        &mut full,
        user,
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info,
        },
        Vec::new(),
    );

    let _ = deposit(&mut light, &mut full, user, &pair.0, 150);
    let _ = deposit(&mut light, &mut full, user, &pair.1, 200);

    let order_specs = [
        ("escape-ask-1", 10, Some(10)),
        ("escape-ask-2", 20, Some(12)),
        ("escape-ask-3", 30, Some(14)),
    ];

    for (order_id, quantity, price) in order_specs {
        submit_signed_order(
            &mut light,
            &mut full,
            &users,
            &signers,
            user,
            Order {
                order_id: order_id.to_string(),
                order_type: OrderType::Limit,
                order_side: OrderSide::Ask,
                price,
                pair: pair.clone(),
                quantity,
            },
        );
    }

    assert_eq!(light.order_manager.orders.len(), 3);
    assert_eq!(full.state.order_manager.orders.len(), 3);

    let light_user_info = light.get_user_info(user).expect("light user info");
    let full_user_info = full.state.get_user_info(user).expect("full user info");

    // Get user balances before escape to create proper transfer blobs
    let light_base_balance = light.get_balance(&light_user_info, &pair.0);
    let light_quote_balance = light.get_balance(&light_user_info, &pair.1);

    // Create transfer blobs for each asset with non-zero balance
    use hyli_smt_token::SmtTokenAction;
    use sdk::{BlobIndex, TxHash};

    let mut blobs = Vec::new();

    // Add transfer blob for base asset if balance > 0
    if light_base_balance.0 > 0 {
        let transfer_blob = SmtTokenAction::Transfer {
            sender: Identity(ORDERBOOK_ACCOUNT_IDENTITY.to_string()),
            recipient: Identity(user.to_string()),
            amount: 150, // Deposited amount
        }
        .as_blob(ContractName(pair.0.clone()), None, None);
        blobs.push(transfer_blob);
    }

    // Add transfer blob for quote asset if balance > 0
    if light_quote_balance.0 > 0 {
        let transfer_blob = SmtTokenAction::Transfer {
            sender: Identity(ORDERBOOK_ACCOUNT_IDENTITY.to_string()),
            recipient: Identity(user.to_string()),
            amount: 200, // Deposited amount
        }
        .as_blob(ContractName(pair.1.clone()), None, None);
        blobs.push(transfer_blob);
    }

    let escape_ctx = TxContext {
        lane_id,
        block_height: BlockHeight(full.last_block_number.0 + 5_001),
        ..Default::default()
    };

    let calldata = sdk::Calldata {
        identity: Identity(ORDERBOOK_ACCOUNT_IDENTITY.to_string()),
        tx_blob_count: blobs.len(),
        blobs: blobs.into(),
        index: BlobIndex(0),
        tx_hash: TxHash::from("escape-test-tx"),
        tx_ctx: Some(escape_ctx),
        private_input: Vec::new(),
    };

    let last_block_number = full.last_block_number;

    let events_light = light
        .escape(&last_block_number, &calldata, &light_user_info)
        .expect("light escape should succeed");
    light
        .apply_events(&light_user_info, &events_light)
        .expect("Could not apply light escape events");

    let events_full = full
        .state
        .escape(&last_block_number, &calldata, &full_user_info)
        .expect("full escape should succeed");
    full.state
        .apply_events(&full_user_info, &events_full)
        .expect("Could not apply full escape events");

    let cancelled_events_light = events_light
        .iter()
        .filter(|event| matches!(event, OrderbookEvent::OrderCancelled { .. }))
        .count();
    let cancelled_events_full = events_full
        .iter()
        .filter(|event| matches!(event, OrderbookEvent::OrderCancelled { .. }))
        .count();

    assert_eq!(cancelled_events_light, 3);
    assert_eq!(cancelled_events_full, 3);

    assert!(light.order_manager.orders.is_empty());
    assert!(full.state.order_manager.orders.is_empty());
    assert!(light.order_manager.orders_owner.is_empty());
    assert!(full.state.order_manager.orders_owner.is_empty());

    assert_eq!(light.get_balance(&light_user_info, &pair.0).0, 0);
    assert_eq!(light.get_balance(&light_user_info, &pair.1).0, 0);
    assert_eq!(full.state.get_balance(&full_user_info, &pair.0).0, 0);
    assert_eq!(full.state.get_balance(&full_user_info, &pair.1).0, 0);
}
