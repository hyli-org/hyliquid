use std::collections::HashSet;
use std::hash::Hash;
use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::{BorshableMerkleProof, SHA256Hasher};
use sparse_merkle_tree::{
    default_store::DefaultStore, error::Result as SmtResult, traits::Value, SparseMerkleTree,
};

type TreeH256 = sparse_merkle_tree::H256;

#[derive(Default, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub struct BorshableH256(pub TreeH256);

impl From<TreeH256> for BorshableH256 {
    fn from(h: TreeH256) -> Self {
        BorshableH256(h)
    }
}

impl From<BorshableH256> for TreeH256 {
    fn from(h: BorshableH256) -> Self {
        h.0
    }
}

impl Value for BorshableH256 {
    fn to_h256(&self) -> TreeH256 {
        self.0
    }

    fn zero() -> Self {
        BorshableH256(TreeH256::zero())
    }
}

impl std::hash::Hash for BorshableH256 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let hash_value = u64::from_le_bytes(self.0.as_slice()[..8].try_into().unwrap());
        state.write_u64(hash_value);
    }
}

impl std::fmt::Debug for BorshableH256 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BorshableH256({})", hex::encode(self.0.as_slice()))
    }
}

impl BorshSerialize for BorshableH256 {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let bytes: [u8; 32] = self.0.into();
        writer.write_all(&bytes)
    }
}

impl BorshDeserialize for BorshableH256 {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut bytes = [0u8; 32];
        reader.read_exact(&mut bytes)?;
        Ok(BorshableH256(TreeH256::from(bytes)))
    }
}

impl std::ops::Deref for BorshableH256 {
    type Target = TreeH256;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for BorshableH256 {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl From<[u8; 32]> for BorshableH256 {
    fn from(bytes: [u8; 32]) -> Self {
        BorshableH256(bytes.into())
    }
}

pub type H256 = BorshableH256;

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub enum Proof {
    Some(BorshableMerkleProof),
    CurrentRootHash(H256),
}

impl Default for Proof {
    fn default() -> Self {
        Proof::CurrentRootHash(H256::zero())
    }
}

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct ZkWitnessSet<
    T: BorshDeserialize + BorshSerialize + Default + Value + GetKey + Ord + Hash + Clone,
> {
    pub values: HashSet<T>,
    pub proof: Proof,
}

impl<T: BorshDeserialize + BorshSerialize + Default + Value + GetKey + Ord + Hash + Clone> Default
    for ZkWitnessSet<T>
{
    fn default() -> Self {
        Self {
            values: HashSet::new(),
            proof: Proof::default(),
        }
    }
}

impl<T: BorshDeserialize + BorshSerialize + Default + Value + GetKey + Ord + Hash + Clone>
    ZkWitnessSet<T>
{
    pub fn compute_root(&self) -> Result<H256, String> {
        match &self.proof {
            Proof::CurrentRootHash(root_hash) => Ok(*root_hash),
            Proof::Some(proof) => {
                let leaves: Vec<(_, _)> = self
                    .values
                    .clone()
                    .into_iter()
                    .map(|v| (v.get_key().into(), v.to_h256()))
                    .collect();

                if leaves.is_empty() {
                    return Err("No leaves in witness set, proof should be empty".to_string());
                }

                let derived_root = proof
                    .0
                    .clone()
                    .compute_root::<SHA256Hasher>(leaves)
                    .map_err(|e| format!("Failed to compute witness proof root: {e}"))?;

                Ok(H256::from(derived_root))
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct SMT<T: Value + Clone>(
    SparseMerkleTree<SHA256Hasher, TreeH256, DefaultStore<TreeH256>>,
    PhantomData<T>,
);

impl<T> Clone for SMT<T>
where
    T: Value + Clone,
{
    fn clone(&self) -> Self {
        let root = *self.0.root();
        let store = self.0.store().clone();
        SMT(SparseMerkleTree::new(root, store), PhantomData)
    }
}

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

    pub fn from_store(root: BorshableH256, store: DefaultStore<TreeH256>) -> Self {
        SMT(SparseMerkleTree::new(root.into(), store), PhantomData)
    }

    pub fn update_all_from_ref<'a, I>(&mut self, leaves: I) -> SmtResult<BorshableH256>
    where
        I: Iterator<Item = &'a T>,
        T: Value + GetKey + 'a,
    {
        let h256_leaves = leaves.map(|el| (el.get_key().0, el.to_h256())).collect();
        self.0.update_all(h256_leaves).map(|r| BorshableH256(*r))
    }

    pub fn update_all<I>(&mut self, leaves: I) -> SmtResult<BorshableH256>
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

    pub fn store(&self) -> &DefaultStore<TreeH256> {
        self.0.store()
    }

    pub fn merkle_proof<'a, I, V>(
        &self,
        keys: I,
    ) -> SmtResult<sparse_merkle_tree::merkle_proof::MerkleProof>
    where
        I: Iterator<Item = &'a V>,
        V: Value + GetKey + 'a,
    {
        self.0
            .merkle_proof(keys.map(|v| v.get_key().0).collect::<Vec<_>>())
    }
}

pub trait GetKey {
    fn get_key(&self) -> BorshableH256;
}

pub trait GetHashMapIndex<K> {
    fn hash_map_index(&self) -> &K;
}
