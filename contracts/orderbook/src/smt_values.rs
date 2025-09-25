use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparse_merkle_tree::{traits::Value, H256};

#[derive(
    Debug, Default, Clone, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct Balance(pub u64);

impl Value for Balance {
    fn to_h256(&self) -> H256 {
        if self.0 == 0 {
            return H256::zero();
        }
        let serialized = borsh::to_vec(self).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&serialized);
        let result = hasher.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&result);
        H256::from(h)
    }

    fn zero() -> Self {
        Balance(0)
    }
}

#[derive(BorshSerialize, BorshDeserialize, Default, Debug, Clone, Eq, PartialEq)]
pub struct UserInfo {
    pub user: String,
    pub salt: Vec<u8>,
    pub nonce: u32,
    pub session_keys: Vec<Vec<u8>>,
}

/// Custom implementation of Ord and PartialOrd
/// WARNING: This does not consider nonce or session_keys. Beware of unexpected behaviours
/// FIXME: Is this shit ? yes
impl Ord for UserInfo {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.user.cmp(&other.user) {
            std::cmp::Ordering::Equal => self.salt.cmp(&other.salt),
            ord => ord,
        }
    }
}

impl PartialOrd for UserInfo {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl UserInfo {
    pub fn new(user: String, salt: Vec<u8>) -> Self {
        UserInfo {
            user,
            salt,
            nonce: 0,
            session_keys: Vec::new(),
        }
    }

    pub fn get_key(&self) -> H256 {
        UserInfo::compute_key(&self.user, &self.salt)
    }

    pub fn compute_key(user: &str, salt: &[u8]) -> H256 {
        let mut hasher = Sha256::new();
        hasher.update(user.as_bytes());
        hasher.update(salt);
        let result = hasher.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&result);
        H256::from(h)
    }

    pub fn orderbook_user_info() -> Self {
        UserInfo {
            user: "orderbook".to_string(),
            salt: vec![],
            nonce: 0,
            session_keys: Vec::new(),
        }
    }
}

impl Value for UserInfo {
    fn to_h256(&self) -> H256 {
        let serialized = borsh::to_vec(self).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&serialized);
        let result = hasher.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&result);
        H256::from(h)
    }

    fn zero() -> Self {
        UserInfo {
            user: String::new(),
            salt: Vec::new(),
            nonce: 0,
            session_keys: Vec::new(),
        }
    }
}
