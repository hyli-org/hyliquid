use borsh::{BorshDeserialize, BorshSerialize};
use sha2::{Digest, Sha256};
use sparse_merkle_tree::traits::Value;
use sparse_merkle_tree::H256;
use state_core::{BorshableH256, GetHashMapIndex, GetKey, Proof};
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
enum Action {
    InsertRecord(DummyLeaf),
    SetMetadata { key: String, value: String },
    SetLevel(u8),
    UpsertBucket { bucket: String, leaf: DummyLeaf },
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
enum Event {
    RecordInserted(DummyLeaf),
    MetadataSet { key: String, value: String },
    LevelSet(u8),
    BucketUpsert { bucket: String, leaf: DummyLeaf },
}

#[vapp_state(action = Action, event = Event)]
struct TestApp {
    #[commit(SMT)]
    records: HashMap<String, DummyLeaf>,

    #[ident(borsh)]
    metadata: HashMap<String, String>,

    level: u8,
}

impl testapp::Logic for testapp::ExecuteState {
    fn compute_events(&self, action: &testapp::Action) -> Vec<testapp::Event> {
        match action {
            testapp::Action::InsertRecord(leaf) => {
                vec![testapp::Event::RecordInserted(leaf.clone())]
            }
            testapp::Action::SetMetadata { key, value } => vec![testapp::Event::MetadataSet {
                key: key.clone(),
                value: value.clone(),
            }],
            testapp::Action::SetLevel(level) => {
                vec![testapp::Event::LevelSet(*level)]
            }
            testapp::Action::UpsertBucket { .. } => Vec::new(),
        }
    }

    fn apply_events(&mut self, events: &[testapp::Event]) {
        for event in events {
            match event {
                testapp::Event::RecordInserted(leaf) => {
                    self.records.insert(leaf.key.clone(), leaf.clone());
                }
                testapp::Event::MetadataSet { key, value } => {
                    self.metadata.insert(key.clone(), value.clone());
                }
                testapp::Event::LevelSet(level) => {
                    self.level = *level;
                }
                testapp::Event::BucketUpsert { .. } => {}
            }
        }
    }
}

#[vapp_state(action = Action, event = Event)]
struct NestedApp {
    #[commit(SMT)]
    buckets: HashMap<String, HashMap<String, DummyLeaf>>,
}

impl nestedapp::Logic for nestedapp::ExecuteState {
    fn compute_events(&self, action: &nestedapp::Action) -> Vec<nestedapp::Event> {
        match action {
            nestedapp::Action::UpsertBucket { bucket, leaf } => {
                vec![nestedapp::Event::BucketUpsert {
                    bucket: bucket.clone(),
                    leaf: leaf.clone(),
                }]
            }
            nestedapp::Action::InsertRecord(_)
            | nestedapp::Action::SetMetadata { .. }
            | nestedapp::Action::SetLevel(_) => Vec::new(),
        }
    }

    fn apply_events(&mut self, events: &[nestedapp::Event]) {
        for event in events {
            match event {
                nestedapp::Event::BucketUpsert { bucket, leaf } => {
                    self.buckets
                        .entry(bucket.clone())
                        .or_default()
                        .insert(leaf.key.clone(), leaf.clone());
                }
                nestedapp::Event::RecordInserted(_)
                | nestedapp::Event::MetadataSet { .. }
                | nestedapp::Event::LevelSet(_) => {}
            }
        }
    }
}

#[test]
fn build_witness_state_collects_single_commit_field() {
    use testapp::Logic as TestLogic;

    let mut full = testapp::FullState::default();

    let alpha = DummyLeaf::new("alpha", 10);
    let beta = DummyLeaf::new("beta", 25);

    let events = vec![
        testapp::Event::RecordInserted(alpha.clone()),
        testapp::Event::RecordInserted(beta.clone()),
        testapp::Event::MetadataSet {
            key: "version".into(),
            value: "1".into(),
        },
        testapp::Event::LevelSet(7),
    ];

    full.apply_events(&events);

    let zk_state = full.build_witness_state(&[]);

    assert_eq!(full.execute_state.records.len(), 2);
    assert_eq!(full.execute_state.level, 7);
    assert_eq!(
        full.execute_state.metadata.get("version"),
        Some(&"1".to_string())
    );

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
    use nestedapp::Logic as NestedLogic;

    let mut full = nestedapp::FullState::default();

    let eth_alice = DummyLeaf::new("alice", 50);
    let eth_bob = DummyLeaf::new("bob", 75);
    let sol_carla = DummyLeaf::new("carla", 20);

    let events = vec![
        nestedapp::Event::BucketUpsert {
            bucket: "ETH".into(),
            leaf: eth_alice.clone(),
        },
        nestedapp::Event::BucketUpsert {
            bucket: "ETH".into(),
            leaf: eth_bob.clone(),
        },
        nestedapp::Event::BucketUpsert {
            bucket: "SOL".into(),
            leaf: sol_carla.clone(),
        },
    ];

    full.apply_events(&events);

    let zk_state = full.build_witness_state(&[]);

    assert_eq!(
        full.execute_state
            .buckets
            .get("ETH")
            .map(|m| m.len())
            .unwrap_or_default(),
        2
    );
    assert_eq!(
        full.execute_state
            .buckets
            .get("SOL")
            .map(|m| m.len())
            .unwrap_or_default(),
        1
    );

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

#[test]
fn full_state_execute_state_matches_standalone_execution() {
    use nestedapp::Logic as NestedLogic;
    use testapp::Logic as TestLogic;

    let alpha = DummyLeaf::new("alpha", 10);
    let beta = DummyLeaf::new("beta", 25);
    let version_event = testapp::Event::MetadataSet {
        key: "version".into(),
        value: "1".into(),
    };
    let level_event = testapp::Event::LevelSet(3);

    let record_events = vec![
        testapp::Event::RecordInserted(alpha.clone()),
        testapp::Event::RecordInserted(beta.clone()),
        version_event.clone(),
        level_event.clone(),
    ];

    let mut expected_exec = testapp::ExecuteState::default();
    expected_exec.apply_events(&record_events);

    let mut full = testapp::FullState::default();
    full.apply_events(&record_events);

    assert_eq!(full.execute_state.records, expected_exec.records);
    assert_eq!(full.execute_state.metadata, expected_exec.metadata);
    assert_eq!(full.execute_state.level, expected_exec.level);

    let nested_events = vec![
        nestedapp::Event::BucketUpsert {
            bucket: "A".into(),
            leaf: DummyLeaf::new("alice", 10),
        },
        nestedapp::Event::BucketUpsert {
            bucket: "A".into(),
            leaf: DummyLeaf::new("bob", 15),
        },
        nestedapp::Event::BucketUpsert {
            bucket: "B".into(),
            leaf: DummyLeaf::new("carla", 20),
        },
    ];

    let mut expected_nested_exec = nestedapp::ExecuteState::default();
    expected_nested_exec.apply_events(&nested_events);

    let mut nested_full = nestedapp::FullState::default();
    nested_full.apply_events(&nested_events);

    assert_eq!(
        nested_full.execute_state.buckets,
        expected_nested_exec.buckets
    );
}

#[test]
fn full_state_action_builds_consistent_witness() {
    use testapp::Logic as TestLogic;

    let mut full = testapp::FullState::default();

    let init_events = vec![
        testapp::Event::RecordInserted(DummyLeaf::new("alpha", 10)),
        testapp::Event::MetadataSet {
            key: "version".into(),
            value: "1".into(),
        },
        testapp::Event::LevelSet(1),
    ];
    full.apply_events(&init_events);

    let new_record = DummyLeaf::new("charlie", 30);
    let action = testapp::Action::InsertRecord(new_record.clone());
    let events = full.apply_action(&action);

    assert_eq!(events.len(), 1);

    let witness = full.build_witness_state(&events);

    assert!(
        witness.records.values.contains(&new_record),
        "witness set should contain inserted record"
    );
    assert_eq!(
        witness.metadata, full.execute_state.metadata,
        "metadata should match"
    );
    assert_eq!(
        witness.level, full.execute_state.level,
        "level should match"
    );

    match witness.records.proof {
        Proof::CurrentRootHash(root) => assert_eq!(root, full.records.root()),
        _ => panic!("expected current root hash proof for records"),
    }
}

#[test]
fn commit_methods_produce_roots_and_refs() {
    use nestedapp::Logic as NestedLogic;
    use testapp::Logic as TestLogic;

    let mut full = testapp::FullState::default();
    let events = vec![
        testapp::Event::RecordInserted(DummyLeaf::new("alpha", 10)),
        testapp::Event::RecordInserted(DummyLeaf::new("beta", 20)),
        testapp::Event::MetadataSet {
            key: "version".into(),
            value: "1".into(),
        },
        testapp::Event::LevelSet(5),
    ];
    full.apply_events(&events);

    let commitment = full.commit();
    assert_eq!(commitment.records, full.records.root());
    assert_eq!(commitment.metadata, &full.metadata);
    assert_eq!(commitment.level, &full.level);

    let witness_state = full.build_witness_state(&events);
    let witness_commitment = witness_state.commit();
    assert_eq!(witness_commitment, commitment);

    let mut nested_full = nestedapp::FullState::default();
    let nested_events = vec![
        nestedapp::Event::BucketUpsert {
            bucket: "ETH".into(),
            leaf: DummyLeaf::new("alice", 50),
        },
        nestedapp::Event::BucketUpsert {
            bucket: "ETH".into(),
            leaf: DummyLeaf::new("bob", 75),
        },
        nestedapp::Event::BucketUpsert {
            bucket: "SOL".into(),
            leaf: DummyLeaf::new("carla", 30),
        },
    ];
    nested_full.apply_events(&nested_events);

    let nested_commitment = nested_full.commit();
    assert_eq!(
        nested_commitment
            .buckets
            .get("ETH")
            .copied()
            .expect("ETH root"),
        nested_full.buckets.get("ETH").expect("ETH tree").root()
    );
    assert_eq!(
        nested_commitment
            .buckets
            .get("SOL")
            .copied()
            .expect("SOL root"),
        nested_full.buckets.get("SOL").expect("SOL tree").root()
    );

    let nested_witness = nested_full.build_witness_state(&nested_events);
    let nested_witness_commit = nested_witness.commit();
    assert_eq!(nested_witness_commit, nested_commitment);
}
