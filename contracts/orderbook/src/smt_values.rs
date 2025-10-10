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

#[derive(
    BorshSerialize, BorshDeserialize, Default, Debug, Clone, Eq, PartialEq, Ord, PartialOrd,
)]
pub struct UserInfo {
    pub user: String,
    pub salt: Vec<u8>,
    pub nonce: u32,
    pub session_keys: Vec<Vec<u8>>,
}

impl std::hash::Hash for UserInfo {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // to_h256() already returns a SHA256 hash, we directly use the first 8 bytes
        // instead of re-hashing the entire content
        let h256 = self.to_h256();
        let bytes = h256.as_slice();
        let hash_value = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        state.write_u64(hash_value);
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

    pub fn get_key(&self) -> BorshableH256 {
        let mut hasher = Sha256::new();
        hasher.update(self.user.as_bytes());
        hasher.update(&self.salt);
        let result = hasher.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&result);
        BorshableH256::from(h)
    }
}

impl Value for UserInfo {
    fn to_h256(&self) -> H256 {
        if self.nonce == 0 {
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
        UserInfo {
            user: String::new(),
            salt: Vec::new(),
            nonce: 0,
            session_keys: Vec::new(),
        }
    }
}

#[derive(Default, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub struct BorshableH256(pub H256);

impl Value for BorshableH256 {
    fn to_h256(&self) -> H256 {
        self.0
    }

    fn zero() -> Self {
        BorshableH256(H256::zero())
    }
}

impl std::hash::Hash for BorshableH256 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // BorshableH256 is already a hash, we directly use the first 8 bytes
        // instead of re-hashing the entire content
        let hash_value = u64::from_le_bytes(self.0.as_slice()[..8].try_into().unwrap());
        state.write_u64(hash_value);
    }
}

impl std::fmt::Debug for BorshableH256 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BorshableH256({})", hex::encode(self.0.as_slice()))
    }
}

impl borsh::BorshSerialize for BorshableH256 {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let bytes: [u8; 32] = self.0.into();
        writer.write_all(&bytes)
    }
}

impl borsh::BorshDeserialize for BorshableH256 {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut bytes = [0u8; 32];
        reader.read_exact(&mut bytes)?;
        Ok(BorshableH256(H256::from(bytes)))
    }
}

impl std::ops::Deref for BorshableH256 {
    type Target = H256;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for BorshableH256 {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl BorshableH256 {
    pub fn as_h256(&self) -> H256 {
        self.0
    }
}

impl From<[u8; 32]> for BorshableH256 {
    fn from(bytes: [u8; 32]) -> Self {
        BorshableH256(bytes.into())
    }
}

impl From<&H256> for BorshableH256 {
    fn from(h: &H256) -> Self {
        BorshableH256(*h)
    }
}

impl<'a> From<&'a H256> for &'a BorshableH256 {
    fn from(h: &'a H256) -> &'a BorshableH256 {
        // SAFETY: This is only safe if the memory layout of H256 and BorshableH256 is the same.
        unsafe { &*(h as *const H256 as *const BorshableH256) }
    }
}

impl From<BorshableH256> for [u8; 32] {
    fn from(h: BorshableH256) -> Self {
        h.0.into()
    }
}

impl From<H256> for BorshableH256 {
    fn from(h: H256) -> Self {
        BorshableH256(h)
    }
}

impl From<BorshableH256> for H256 {
    fn from(h: BorshableH256) -> Self {
        h.0
    }
}
