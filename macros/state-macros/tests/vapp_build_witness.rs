use borsh::{BorshDeserialize, BorshSerialize};
use sha2::{Digest, Sha256};
use sparse_merkle_tree::traits::Value;
use sparse_merkle_tree::H256;
use state_core::{BorshableH256, GetHashMapIndex, GetKey, Proof, SMT};
use state_macros::vapp_state;
use std::collections::HashMap;

#[derive(
    Debug, Clone, Default, Eq, PartialEq, Ord, PartialOrd, Hash, BorshSerialize, BorshDeserialize,
)]
struct DummyLeaf {
    key: String,
    value: u64,
}

impl DummyLeaf {
    fn new(key: &str, value: u64) -> Self {
        Self {
            key: key.to_string(),
            value,
        }
    }
}

impl GetHashMapIndex<String> for DummyLeaf {
    fn hash_map_index(&self) -> &String {
        &self.key
    }
}

impl GetKey for DummyLeaf {
    fn get_key(&self) -> BorshableH256 {
        let mut hasher = Sha256::new();
        hasher.update(self.key.as_bytes());
        let digest = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&digest);
        BorshableH256::from(bytes)
    }
}

impl Value for DummyLeaf {
    fn to_h256(&self) -> H256 {
        if self.value == 0 {
            return H256::zero();
        }
        let mut hasher = Sha256::new();
        hasher.update(self.key.as_bytes());
        hasher.update(self.value.to_le_bytes());
        let digest = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&digest);
        H256::from(bytes)
    }

    fn zero() -> Self {
        Self::default()
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
enum Action {}

#[allow(dead_code)]
#[derive(Debug, Clone)]
enum Event {}

#[vapp_state(action = Action, event = Event)]
struct TestApp {
    #[commit(SMT)]
    records: HashMap<String, DummyLeaf>,

    #[ident(borsh)]
    metadata: HashMap<String, String>,

    level: u8,
}

#[allow(dead_code)]
impl testapp::ExecuteState {
    pub fn compute_events_logic(&self, _action: &testapp::Action) -> Vec<testapp::Event> {
        Vec::new()
    }

    pub fn apply_events_logic(&mut self, _events: &[testapp::Event]) {}
}

#[vapp_state(action = Action, event = Event)]
struct NestedApp {
    #[commit(SMT)]
    buckets: HashMap<String, HashMap<String, DummyLeaf>>,
}

#[allow(dead_code)]
impl nestedapp::ExecuteState {
    pub fn compute_events_logic(&self, _action: &nestedapp::Action) -> Vec<nestedapp::Event> {
        Vec::new()
    }

    pub fn apply_events_logic(&mut self, _events: &[nestedapp::Event]) {}
}

#[test]
fn build_witness_state_collects_single_commit_field() {
    let mut full = testapp::FullState {
        execute_state: testapp::ExecuteState::default(),
        records: SMT::zero(),
        metadata: HashMap::new(),
        level: 0,
    };

    let alpha = DummyLeaf::new("alpha", 10);
    let beta = DummyLeaf::new("beta", 25);

    full.execute_state
        .records
        .insert(alpha.key.clone(), alpha.clone());
    full.execute_state
        .records
        .insert(beta.key.clone(), beta.clone());
    full.execute_state
        .metadata
        .insert("version".into(), "1".into());

    full.execute_state.level = 7;

    full.sync_commitments();

    let zk_state = full.build_witness_state(&[]);

    assert_eq!(zk_state.metadata, full.execute_state.metadata);
    assert_eq!(zk_state.level, full.execute_state.level);

    assert_eq!(zk_state.records.values.len(), 2);
    assert!(zk_state.records.values.contains(&alpha));
    assert!(zk_state.records.values.contains(&beta));

    match zk_state.records.proof {
        Proof::CurrentRootHash(root) => assert_eq!(root, full.records.root()),
        _ => panic!("expected current root hash proof"),
    }
}

#[test]
fn build_witness_state_collects_nested_commit_fields() {
    let mut full = nestedapp::FullState::default();

    let eth_alice = DummyLeaf::new("alice", 50);
    let eth_bob = DummyLeaf::new("bob", 75);
    let sol_carla = DummyLeaf::new("carla", 20);

    {
        let balances = full.execute_state.buckets.entry("ETH".into()).or_default();
        balances.insert(eth_alice.key.clone(), eth_alice.clone());
        balances.insert(eth_bob.key.clone(), eth_bob.clone());
    }

    {
        let balances = full.execute_state.buckets.entry("SOL".into()).or_default();
        balances.insert(sol_carla.key.clone(), sol_carla.clone());
    }

    full.sync_commitments();

    let zk_state = full.build_witness_state(&[]);

    assert_eq!(zk_state.buckets.len(), 2);

    let eth_witness = zk_state.buckets.get("ETH").expect("missing ETH witness");
    assert_eq!(eth_witness.values.len(), 2);
    assert!(eth_witness.values.contains(&eth_alice));
    assert!(eth_witness.values.contains(&eth_bob));

    match eth_witness.proof {
        Proof::CurrentRootHash(root) => {
            let tree = full.buckets.get("ETH").expect("missing ETH tree");
            assert_eq!(root, tree.root());
        }
        _ => panic!("expected current root hash proof for ETH"),
    }

    let sol_witness = zk_state.buckets.get("SOL").expect("missing SOL witness");
    assert_eq!(sol_witness.values.len(), 1);
    assert!(sol_witness.values.contains(&sol_carla));

    match sol_witness.proof {
        Proof::CurrentRootHash(root) => {
            let tree = full.buckets.get("SOL").expect("missing SOL tree");
            assert_eq!(root, tree.root());
        }
        _ => panic!("expected current root hash proof for SOL"),
    }
}
