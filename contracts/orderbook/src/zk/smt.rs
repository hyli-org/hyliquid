use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use sha3::{Digest, Sha3_256};
use sparse_merkle_tree::{
    default_store::DefaultStore,
    traits::{Hasher, Value},
    SparseMerkleTree, H256,
};

use crate::{
    model::{Balance, Order, OrderSide, OrderType, UserInfo},
    zk::order_merkle::OrderPriceLevel,
};

#[derive(
    Debug, Default, Clone, BorshSerialize, BorshDeserialize, Eq, PartialEq, PartialOrd, Ord, Hash,
)]
pub struct UserBalance {
    pub user_key: BorshableH256,
    pub balance: Balance,
}

impl Value for UserBalance {
    fn to_h256(&self) -> H256 {
        if self.balance.0 == 0 {
            return H256::zero();
        }
        let serialized = borsh::to_vec(&self.balance).unwrap();
        let mut hasher = Sha3_256::new();
        hasher.update(&serialized);
        let result = hasher.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&result);
        H256::from(h)
    }

    fn zero() -> Self {
        UserBalance {
            user_key: BorshableH256(H256::zero()),
            balance: Balance(0),
        }
    }
}

impl GetKey for UserBalance {
    fn get_key(&self) -> BorshableH256 {
        self.user_key
    }
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
}

pub trait GetKey {
    fn get_key(&self) -> BorshableH256;
}

impl GetKey for UserInfo {
    fn get_key(&self) -> BorshableH256 {
        let mut hasher = Sha3_256::new();
        hasher.update(self.user.as_bytes());
        hasher.update(&self.salt);
        let result = hasher.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&result);
        BorshableH256::from(h)
    }
}

impl<T: GetKey> GetKey for &T {
    fn get_key(&self) -> BorshableH256 {
        (*self).get_key()
    }
}

impl Value for UserInfo {
    fn to_h256(&self) -> H256 {
        if self.nonce == 0 {
            return H256::zero();
        }

        let serialized = borsh::to_vec(self).unwrap();
        let mut hasher = Sha3_256::new();
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

impl GetKey for Order {
    fn get_key(&self) -> BorshableH256 {
        let mut hasher = Sha3_256::new();
        hasher.update(self.order_id.as_bytes());
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        BorshableH256::from(bytes)
    }
}

impl Value for Order {
    fn to_h256(&self) -> H256 {
        if self.quantity == 0 {
            return H256::zero();
        }

        let serialized =
            borsh::to_vec(self).expect("Order should serialize for Merkle tree hashing");
        let mut hasher = Sha3_256::new();
        hasher.update(&serialized);
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        H256::from(bytes)
    }

    fn zero() -> Self {
        Order {
            order_id: String::new(),
            order_type: OrderType::Limit,
            order_side: OrderSide::Bid,
            price: None,
            pair: (String::new(), String::new()),
            quantity: 0,
        }
    }
}

impl GetKey for OrderPriceLevel {
    fn get_key(&self) -> BorshableH256 {
        let mut hasher = Sha3_256::new();
        hasher.update(self.pair.0.as_bytes());
        hasher.update(self.pair.1.as_bytes());
        hasher.update(self.price.to_le_bytes());
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        BorshableH256::from(bytes)
    }
}

impl Value for OrderPriceLevel {
    fn to_h256(&self) -> H256 {
        if self.order_ids.is_empty() {
            return H256::zero();
        }

        let serialized =
            borsh::to_vec(self).expect("OrderPriceLevel should serialize for Merkle tree hashing");
        let mut hasher = Sha3_256::new();
        hasher.update(&serialized);
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        H256::from(bytes)
    }

    fn zero() -> Self {
        OrderPriceLevel {
            pair: (String::new(), String::new()),
            price: 0,
            order_ids: Vec::new(),
        }
    }
}

#[derive(Default, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub struct BorshableH256(pub H256);

impl serde::Serialize for BorshableH256 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let bytes: [u8; 32] = self.0.into();
        serializer.serialize_bytes(&bytes)
    }
}

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

#[derive(Debug, Default)]
pub struct SMT<T: Value + Clone>(
    SparseMerkleTree<SHA3_256Hasher, H256, DefaultStore<H256>>,
    PhantomData<T>,
);

impl<T> SMT<T>
where
    T: Value + Clone,
{
    pub fn zero() -> Self {
        SMT(
            SparseMerkleTree::new(sparse_merkle_tree::H256::zero(), Default::default()),
            PhantomData,
        )
    }

    pub fn from_store(root: BorshableH256, store: DefaultStore<H256>) -> Self {
        SMT(SparseMerkleTree::new(root.into(), store), PhantomData)
    }

    pub fn update_all_from_ref<'a, I>(
        &mut self,
        leaves: I,
    ) -> sparse_merkle_tree::error::Result<BorshableH256>
    where
        I: Iterator<Item = &'a T>,
        T: Value + GetKey + 'a,
    {
        let h256_leaves = leaves.map(|el| (el.get_key().0, el.to_h256())).collect();
        self.0.update_all(h256_leaves).map(|r| BorshableH256(*r))
    }

    pub fn update_all<I>(&mut self, leaves: I) -> sparse_merkle_tree::error::Result<BorshableH256>
    where
        I: Iterator<Item = T>,
        T: Value + GetKey,
    {
        let h256_leaves = leaves.map(|el| (el.get_key().0, el.to_h256())).collect();
        self.0.update_all(h256_leaves).map(|r| BorshableH256(*r))
    }

    pub fn root(&self) -> BorshableH256 {
        BorshableH256(*self.0.root())
    }

    pub fn store(&self) -> &DefaultStore<H256> {
        self.0.store()
    }

    pub fn merkle_proof<'a, I, V>(
        &self,
        keys: I,
    ) -> sparse_merkle_tree::error::Result<sparse_merkle_tree::merkle_proof::MerkleProof>
    where
        I: Iterator<Item = &'a V>,
        V: Value + GetKey + 'a,
    {
        self.0
            .merkle_proof(keys.map(|v| v.get_key().0).collect::<Vec<_>>())
    }
}

// Custom SHA3_256Hasher implementation
#[derive(Default, Debug)]
pub struct SHA3_256Hasher(Sha3_256);

impl Hasher for SHA3_256Hasher {
    fn write_h256(&mut self, h: &H256) {
        self.0.update(h.as_slice());
    }

    fn write_byte(&mut self, b: u8) {
        self.0.update([b]);
    }

    fn finish(self) -> H256 {
        let result = self.0.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&result);
        H256::from(h)
    }
}
