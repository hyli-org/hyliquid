#![cfg(test)]

use std::collections::{BTreeMap, HashSet};

use borsh::BorshDeserialize;
use k256::ecdsa::signature::DigestSigner;
use k256::ecdsa::{Signature, SigningKey};
use sdk::{guest, BlockHeight, LaneId, StateCommitment};
use sdk::{tracing, ContractAction};
use sdk::{BlobIndex, Calldata, ContractName, Identity, TxContext, TxHash};
use sha3::{Digest, Sha3_256};

use crate::model::{
    AssetInfo, ExecuteState, Order, OrderSide, OrderType, OrderbookEvent, Pair, PairInfo, UserInfo,
    WithdrawDestination,
};
use crate::transaction::{
    AddSessionKeyPrivateInput, CancelOrderPrivateInput, CreateOrderPrivateInput, OrderbookAction,
    PermissionedOrderbookAction, PermissionedPrivateInput, WithdrawPrivateInput,
};
use crate::zk::OrderManagerRoots;
use crate::zk::{FullState, ZkVmState, H256};
use crate::ORDERBOOK_ACCOUNT_IDENTITY;

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
    balances_roots: BTreeMap<String, H256>,
    assets: BTreeMap<String, AssetInfo>,
    order_commitment: OrderManagerRoots,
    hashed_secret: [u8; 32],
    lane_id: LaneId,
    last_block_number: BlockHeight,
}

fn run_action(
    light: &mut ExecuteState,
    full: &mut FullState,
    user: &str,
    action: PermissionedOrderbookAction,
    private_payload: Vec<u8>,
) -> Vec<OrderbookEvent> {
    let action_repr = format!("{action:?}");
    let (cn, id, tx_ctx, _, secret) = get_ctx();

    let user_info = light
        .get_user_info(user)
        .unwrap_or_else(|_| test_user(user));

    let events = light
        .execute_permissioned_action(user_info.clone(), action.clone(), &private_payload)
        .expect("light execution");
    light.order_manager.clean(&events);

    tracing::debug!("light events: {:?}", events);

    let commitment_metadata = full
        .derive_zkvm_commitment_metadata_from_events(&user_info, &events, &action)
        .expect("derive metadata");

    let full_initial_commitment = full.commit();

    full.apply_events_and_update_roots(&user_info, events.clone())
        .expect("full execution deposit");

    let permissioned_private_input = PermissionedPrivateInput {
        secret: secret.to_vec(),
        user_info: user_info.clone(),
        private_input: private_payload,
    };

    let calldata = Calldata {
        identity: id.clone(),
        blobs: vec![OrderbookAction::PermissionedOrderbookAction(action, 0).as_blob(cn.clone())]
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
        let err = String::from_utf8_lossy(&hyli_output.program_outputs);
        let known_owners = full
            .state
            .order_manager
            .orders_owner
            .values()
            .collect::<Vec<_>>();
        let metadata_owners = metadata_state
            .order_manager
            .orders_owner
            .keys()
            .collect::<Vec<_>>();
        panic!(
            "execution failed for action {action_repr}: {hyli_output:?}; known owners: {known_owners:?}; metadata owners: {metadata_owners:?}, err: {err}",
        );
    }

    assert_eq!(
        hyli_output.initial_state, full_initial_commitment,
        "Full initial state mismatch for action {action_repr}"
    );
    let full_next_commitment = full.commit();
    assert_eq!(
        hyli_output.next_state, full_next_commitment,
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
    expected: &mut BTreeMap<&'a str, BalanceExpectation>,
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
    expected: &BTreeMap<&'a str, BalanceExpectation>,
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
        PermissionedOrderbookAction::CreateOrder(order),
        private_payload,
    );
}

fn cancel_signed_order<'a>(
    light: &mut ExecuteState,
    full: &mut FullState,
    users: &[&'a str],
    signers: &'a [TestSigner],
    user: &str,
    order_id: &str,
) -> Vec<OrderbookEvent> {
    let signer = signer_for(users, signers, user);
    let user_info = full
        .state
        .get_user_info(user)
        .expect("user info for signature");
    let msg = format!("{}:{}:cancel:{order_id}", user, user_info.nonce);
    let signature = signer.sign(&msg);
    let private_input = CancelOrderPrivateInput {
        signature,
        public_key: signer.public_key.clone(),
    };
    let private_payload = borsh::to_vec(&private_input).expect("serialize cancel order input");

    run_action(
        light,
        full,
        user,
        PermissionedOrderbookAction::Cancel {
            order_id: order_id.to_string(),
        },
        private_payload,
    )
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
        PermissionedOrderbookAction::AddSessionKey,
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
        PermissionedOrderbookAction::Deposit {
            symbol: symbol.to_string(),
            amount,
        },
        Vec::new(),
    )
}

fn withdraw_with_signature<'a>(
    light: &mut ExecuteState,
    full: &mut FullState,
    users: &[&'a str],
    signers: &'a [TestSigner],
    user: &str,
    symbol: &str,
    amount: u64,
) {
    let signer = signer_for(users, signers, user);
    let user_info = full
        .state
        .get_user_info(user)
        .expect("user info before withdraw");
    let msg = format!(
        "{user}:{nonce}:withdraw:{symbol}:{amount}",
        nonce = user_info.nonce
    );
    let signature = signer.sign(&msg);
    let private_input = WithdrawPrivateInput {
        signature,
        public_key: signer.public_key.clone(),
    };
    let private_payload = borsh::to_vec(&private_input).expect("serialize withdraw input");

    let destination = WithdrawDestination {
        network: "testnet".to_string(),
        address: format!("{user}-dest"),
    };

    let _ = run_action(
        light,
        full,
        user,
        PermissionedOrderbookAction::Withdraw {
            symbol: symbol.to_string(),
            amount,
            destination,
        },
        private_payload,
    );
}

fn decode_commitment(commitment: &StateCommitment) -> OwnedCommitment {
    borsh::from_slice(&commitment.0).expect("decode state commitment")
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
        PermissionedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info,
        },
        Vec::new(),
    );

    let deposit_amount = 1_000_u64;
    let _ = deposit(
        &mut light,
        &mut full,
        users[0],
        &base_symbol,
        deposit_amount,
    );
    let commitment = full.commit();
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
        PermissionedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info,
        },
        Vec::new(),
    );

    let first_amount = 1_000_u64;
    let _ = deposit(&mut light, &mut full, users[0], &base_symbol, first_amount);
    let first_commitment = full.commit();
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
    let _ = deposit(&mut light, &mut full, users[0], &base_symbol, second_amount);
    let second_commitment = full.commit();
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
fn test_withdraw_reduces_balance_and_increments_nonce() {
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
        PermissionedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info,
        },
        Vec::new(),
    );

    let deposit_amount = 1_000_u64;
    let _ = deposit(
        &mut light,
        &mut full,
        users[0],
        &base_symbol,
        deposit_amount,
    );

    let before_withdraw_user = full
        .state
        .get_user_info(users[0])
        .expect("user info before withdraw");

    let withdrawn_amount = 400_u64;

    withdraw_with_signature(
        &mut light,
        &mut full,
        &users,
        &signers,
        users[0],
        &base_symbol,
        withdrawn_amount,
    );

    let light_info = light.get_user_info(users[0]).expect("light user info");
    let full_info = full
        .state
        .get_user_info(users[0])
        .expect("full user info after withdraw");
    assert_eq!(
        light.get_balance(&light_info, &base_symbol).0,
        deposit_amount - withdrawn_amount
    );
    assert_eq!(
        full.state.get_balance(&full_info, &base_symbol).0,
        deposit_amount - withdrawn_amount
    );
    assert_eq!(
        full_info.nonce,
        before_withdraw_user.nonce + 1,
        "nonce should increment after withdraw"
    );
}

#[test_log::test]
fn test_limit_order_without_price_fails() {
    let light = ExecuteState::default();
    let user_info = test_user("alice");
    let order = Order {
        order_id: "limit-no-price".to_string(),
        order_type: OrderType::Limit,
        order_side: OrderSide::Ask,
        price: None,
        pair: ("AAA".to_string(), "BBB".to_string()),
        quantity: 10,
    };
    let err = light
        .generate_permissioned_execution_events(
            &user_info,
            PermissionedOrderbookAction::CreateOrder(order),
            &[],
        )
        .expect_err("limit order without price should fail");
    assert_eq!(err, "Limit orders must have a price");
}

#[test_log::test]
fn test_market_order_with_price_fails() {
    let light = ExecuteState::default();
    let user_info = test_user("alice");
    let order = Order {
        order_id: "market-with-price".to_string(),
        order_type: OrderType::Market,
        order_side: OrderSide::Bid,
        price: Some(10),
        pair: ("AAA".to_string(), "BBB".to_string()),
        quantity: 10,
    };
    let err = light
        .generate_permissioned_execution_events(
            &user_info,
            PermissionedOrderbookAction::CreateOrder(order),
            &[],
        )
        .expect_err("market order with price should fail");
    assert_eq!(err, "Market orders cannot have a price");
}

#[test_log::test]
fn test_identify_action_is_noop() {
    let (_, _, _, lane_id, secret) = get_ctx();

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
    let pair_info = PairInfo {
        base: AssetInfo::new(0, ContractName(base_symbol.clone())),
        quote: AssetInfo::new(0, ContractName(quote_symbol.clone())),
    };

    let _ = run_action(
        &mut light,
        &mut full,
        users[0],
        PermissionedOrderbookAction::CreatePair {
            pair: (base_symbol.clone(), quote_symbol.clone()),
            info: pair_info,
        },
        Vec::new(),
    );

    let _ = deposit(&mut light, &mut full, users[0], &base_symbol, 500);

    let before_commit = full.commit();
    let before_user = full
        .state
        .get_user_info(users[0])
        .expect("user info before identify");
    let before_balance = light
        .get_balance(
            &light.get_user_info(users[0]).expect("light user"),
            &base_symbol,
        )
        .0;

    let events = run_action(
        &mut light,
        &mut full,
        users[0],
        PermissionedOrderbookAction::Identify,
        Vec::new(),
    );
    assert!(events.is_empty(), "identify should emit no events");

    let after_commit = full.commit();
    let after_user = full
        .state
        .get_user_info(users[0])
        .expect("user info after identify");
    let after_balance = light
        .get_balance(
            &light.get_user_info(users[0]).expect("light user"),
            &base_symbol,
        )
        .0;

    assert_eq!(
        before_commit, after_commit,
        "state commitment should not change"
    );
    assert_eq!(
        before_user.nonce, after_user.nonce,
        "identify must not bump nonce"
    );
    assert_eq!(
        before_balance, after_balance,
        "balances should remain untouched"
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

    let private_payload = borsh::to_vec(&AddSessionKeyPrivateInput {
        new_public_key: signer.public_key.clone(),
    })
    .expect("serialize add session key input");

    let _ = run_action(
        &mut light,
        &mut full,
        user,
        PermissionedOrderbookAction::AddSessionKey,
        private_payload,
    );

    let final_commitment = full.commit();

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

#[test_log::test]
fn test_equal_price_limit_orders_fill_in_fifo_order() {
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

    let users = ["alice", "bob", "carol"];
    let signers: Vec<TestSigner> = (0..users.len())
        .map(|idx| TestSigner::new((idx + 1) as u8))
        .collect();

    for user in &users {
        add_session_key(&mut light, &mut full, &users, &signers, user);
    }

    let _ = run_action(
        &mut light,
        &mut full,
        users[0],
        PermissionedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info,
        },
        Vec::new(),
    );

    for seller in &users[..2] {
        let _ = deposit(&mut light, &mut full, seller, &pair.0, 1_000);
    }
    let _ = deposit(&mut light, &mut full, users[2], &pair.1, 10_000);

    submit_signed_order(
        &mut light,
        &mut full,
        &users,
        &signers,
        users[0],
        Order {
            order_id: "ask-fifo-1".to_string(),
            order_type: OrderType::Limit,
            order_side: OrderSide::Ask,
            price: Some(10),
            pair: pair.clone(),
            quantity: 30,
        },
    );

    submit_signed_order(
        &mut light,
        &mut full,
        &users,
        &signers,
        users[1],
        Order {
            order_id: "ask-fifo-2".to_string(),
            order_type: OrderType::Limit,
            order_side: OrderSide::Ask,
            price: Some(10),
            pair: pair.clone(),
            quantity: 30,
        },
    );

    {
        let price_level = light
            .order_manager
            .ask_orders
            .get(&pair)
            .and_then(|levels| levels.get(&10))
            .expect("price level after order placement");
        let order_ids: Vec<_> = price_level.iter().cloned().collect();
        assert_eq!(
            order_ids,
            vec!["ask-fifo-1".to_string(), "ask-fifo-2".to_string()],
            "orders should enter the book in insertion order"
        );
    }

    submit_signed_order(
        &mut light,
        &mut full,
        &users,
        &signers,
        users[2],
        Order {
            order_id: "fifo-market-taker".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 40,
        },
    );

    let price_level_after = light
        .order_manager
        .ask_orders
        .get(&pair)
        .and_then(|levels| levels.get(&10))
        .expect("price level after matching");
    let remaining_ids: Vec<_> = price_level_after.iter().cloned().collect();
    assert_eq!(
        remaining_ids,
        vec!["ask-fifo-2".to_string()],
        "secondary order should remain when market order does not clear level"
    );

    assert!(
        !light.order_manager.orders.contains_key("ask-fifo-1"),
        "first order should be fully filled and removed"
    );
    let remaining_order = light
        .order_manager
        .orders
        .get("ask-fifo-2")
        .expect("second order should remain");
    assert_eq!(
        remaining_order.quantity, 20,
        "remaining order quantity should reflect partial fill"
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
    expected: &mut BTreeMap<&'a str, BalanceExpectation>,
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
fn test_cancel_order_restores_balance_and_removes_state() {
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

    let users = ["alice"];
    let signers = vec![TestSigner::new(1)];
    let user = users[0];

    add_session_key(&mut light, &mut full, &users, &signers, user);

    let _ = run_action(
        &mut light,
        &mut full,
        user,
        PermissionedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info,
        },
        Vec::new(),
    );

    let initial_base_deposit = 100_u64;
    let initial_quote_deposit = 1_000_u64;
    let _ = deposit(
        &mut light,
        &mut full,
        user,
        &base_symbol,
        initial_base_deposit,
    );
    let _ = deposit(
        &mut light,
        &mut full,
        user,
        &quote_symbol,
        initial_quote_deposit,
    );

    let light_user_info = light
        .get_user_info(user)
        .expect("light user info after deposit");
    let full_user_info = full
        .state
        .get_user_info(user)
        .expect("full user info after deposit");
    assert_eq!(
        light.get_balance(&light_user_info, &base_symbol).0,
        initial_base_deposit
    );
    assert_eq!(
        light.get_balance(&light_user_info, &quote_symbol).0,
        initial_quote_deposit
    );
    assert_eq!(
        full.state.get_balance(&full_user_info, &base_symbol).0,
        initial_base_deposit
    );
    assert_eq!(
        full.state.get_balance(&full_user_info, &quote_symbol).0,
        initial_quote_deposit
    );

    let ask_order_id = "ask-to-cancel";
    let ask_quantity = 40_u64;
    let ask_price = 12_u64;
    submit_signed_order(
        &mut light,
        &mut full,
        &users,
        &signers,
        user,
        Order {
            order_id: ask_order_id.to_string(),
            order_type: OrderType::Limit,
            order_side: OrderSide::Ask,
            price: Some(ask_price),
            pair: pair.clone(),
            quantity: ask_quantity,
        },
    );

    let light_user_info = light
        .get_user_info(user)
        .expect("light user info after ask");
    let full_user_info = full
        .state
        .get_user_info(user)
        .expect("full user info after ask");
    assert_eq!(
        light.get_balance(&light_user_info, &base_symbol).0,
        initial_base_deposit - ask_quantity
    );
    assert_eq!(
        full.state.get_balance(&full_user_info, &base_symbol).0,
        initial_base_deposit - ask_quantity
    );

    let bid_order_id = "bid-remains";
    let bid_quantity = 10_u64;
    let bid_price = 1_u64;
    submit_signed_order(
        &mut light,
        &mut full,
        &users,
        &signers,
        user,
        Order {
            order_id: bid_order_id.to_string(),
            order_type: OrderType::Limit,
            order_side: OrderSide::Bid,
            price: Some(bid_price),
            pair: pair.clone(),
            quantity: bid_quantity,
        },
    );

    let expected_quote_after_bid = initial_quote_deposit - (bid_quantity * bid_price);
    let light_user_info = light
        .get_user_info(user)
        .expect("light user info after bid");
    let full_user_info = full
        .state
        .get_user_info(user)
        .expect("full user info after bid");
    assert_eq!(
        light.get_balance(&light_user_info, &quote_symbol).0,
        expected_quote_after_bid
    );
    assert_eq!(
        full.state.get_balance(&full_user_info, &quote_symbol).0,
        expected_quote_after_bid
    );

    assert!(
        light.order_manager.orders.contains_key(ask_order_id),
        "ask order should be present before cancellation in light state"
    );
    assert!(
        full.state.order_manager.orders.contains_key(ask_order_id),
        "ask order should be present before cancellation in full state"
    );

    let ask_orders_light = light
        .order_manager
        .ask_orders
        .get(&pair)
        .expect("light should have ask orders before cancellation");
    assert!(
        ask_orders_light
            .values()
            .any(|ids| ids.iter().any(|id| id == ask_order_id)),
        "ask order should be tracked in ask book for light state"
    );
    let ask_orders_full = full
        .state
        .order_manager
        .ask_orders
        .get(&pair)
        .expect("full should have ask orders before cancellation");
    assert!(
        ask_orders_full
            .values()
            .any(|ids| ids.iter().any(|id| id == ask_order_id)),
        "ask order should be tracked in ask book for full state"
    );

    let events = cancel_signed_order(&mut light, &mut full, &users, &signers, user, ask_order_id);

    assert!(
        events.iter().any(|event| matches!(
            event,
            OrderbookEvent::OrderCancelled { order_id, .. } if order_id == ask_order_id
        )),
        "cancellation should emit OrderCancelled event"
    );

    let light_user_info = light
        .get_user_info(user)
        .expect("light user info after cancellation");
    let full_user_info = full
        .state
        .get_user_info(user)
        .expect("full user info after cancellation");
    assert_eq!(
        light.get_balance(&light_user_info, &base_symbol).0,
        initial_base_deposit
    );
    assert_eq!(
        full.state.get_balance(&full_user_info, &base_symbol).0,
        initial_base_deposit
    );
    assert_eq!(
        light.get_balance(&light_user_info, &quote_symbol).0,
        expected_quote_after_bid
    );
    assert_eq!(
        full.state.get_balance(&full_user_info, &quote_symbol).0,
        expected_quote_after_bid
    );

    assert!(
        !light.order_manager.orders.contains_key(ask_order_id),
        "cancelled order should be removed from light state storage"
    );
    assert!(
        !full.state.order_manager.orders.contains_key(ask_order_id),
        "cancelled order should be removed from full state storage"
    );
    assert!(
        light.order_manager.orders.contains_key(bid_order_id),
        "non-cancelled order should remain in light state"
    );
    assert!(
        full.state.order_manager.orders.contains_key(bid_order_id),
        "non-cancelled order should remain in full state"
    );

    if let Some(orders) = light.order_manager.ask_orders.get(&pair) {
        assert!(
            orders.values().all(|ids| ids.is_empty()),
            "ask book should be empty for light state after cancellation"
        );
    }
    if let Some(orders) = full.state.order_manager.ask_orders.get(&pair) {
        assert!(
            orders.values().all(|ids| ids.is_empty()),
            "ask book should be empty for full state after cancellation"
        );
    }
    assert!(
        !light.order_manager.orders_owner.contains_key(ask_order_id),
        "cancelled order owner mapping should be cleared in light state"
    );
    assert!(
        !full
            .state
            .order_manager
            .orders_owner
            .contains_key(ask_order_id),
        "cancelled order owner mapping should be cleared in full state"
    );
    assert!(
        light.order_manager.orders_owner.contains_key(bid_order_id),
        "non-cancelled order owner mapping should remain in light state"
    );
    assert!(
        full.state
            .order_manager
            .orders_owner
            .contains_key(bid_order_id),
        "non-cancelled order owner mapping should remain in full state"
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

    let mut expected_balances: BTreeMap<&str, BalanceExpectation> = users
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
        PermissionedOrderbookAction::CreatePair {
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
            order_id: "market2".to_string(),
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
            order_id: "market3".to_string(),
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
            order_id: "market4".to_string(),
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
            order_id: "market5".to_string(),
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
            order_id: "market6".to_string(),
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
            order_id: "market7".to_string(),
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
            order_id: "market8".to_string(),
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
        "after clearing bid-lim3 and bid-lim4 and partially bid-lim5",
        Order {
            order_id: "market9".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 55,
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
            delta(alice, -qty(10), notional(10, 4)),
            delta(charlie, qty(25), 0),
            delta(alice, qty(20), 0),
            delta(bob, qty(10), 0),
        ],
    );

    // Add a new bid order before market10 to be consumed by it
    submit_signed_order(
        &mut light,
        &mut full,
        &users,
        &signers,
        bob,
        Order {
            order_id: "bid-extra".to_string(),
            order_type: OrderType::Limit,
            order_side: OrderSide::Bid,
            price: Some(2),
            pair: pair.clone(),
            quantity: 12,
        },
    );
    apply_balance_deltas(&mut expected_balances, &[delta(bob, 0, -notional(12, 2))]);

    execute_market_order(
        "after clearing remaining bid orders",
        Order {
            order_id: "market10".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 27, // Increased from 15 to consume the new bid order too
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
            delta(alice, -qty(5), notional(5, 4)),
            delta(alice, -qty(10), notional(10, 3)),
            delta(alice, -qty(12), notional(12, 2)),
            delta(bob, qty(5), 0),
            delta(charlie, qty(10), 0),
            delta(bob, qty(12), 0),
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
        PermissionedOrderbookAction::CreatePair {
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
