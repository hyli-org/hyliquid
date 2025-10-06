use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use monotree::{hasher::Sha2, Monotree};
use monotree::{DefaultDatabase, Hash as MonotreeHash};

use crate::monotree_multi_proof::ProofStatus;
use crate::monotree_proof::compute_root_from_proof;
use crate::orderbook::{OrderbookEvent, TokenName};
use crate::orderbook_witness::ZkVmWitness;
use crate::{
    orderbook::{ExecutionState, Orderbook},
    smt_values::{Balance, BorshableH256 as H256, MonotreeValue, UserInfo},
};

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct LightState {
    pub users_info: HashMap<String, UserInfo>,
    pub balances: HashMap<TokenName, HashMap<H256, Balance>>,
}

#[derive(Default, Debug)]
pub struct FullState {
    pub light: LightState,
    pub users_info_mt: MonotreeCommitment<UserInfo>,
    pub balances_mt: HashMap<TokenName, MonotreeCommitment<Balance>>,
}

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct ZkVmState {
    pub users_info: ZkVmWitness<HashSet<UserInfo>>,
    pub balances: HashMap<TokenName, ZkVmWitness<HashMap<H256, Balance>>>,
}

/// impl of functions for state management
impl Orderbook {
    pub fn fund_account(
        &mut self,
        token: &str,
        user_info: &UserInfo,
        amount: &Balance,
    ) -> Result<(), String> {
        let current_balance = self.get_balance(user_info, token);

        self.update_balances(
            token,
            vec![(user_info.get_key(), Balance(current_balance.0 + amount.0))],
        )
        .map_err(|e| e.to_string())
    }

    pub fn deduct_from_account(
        &mut self,
        token: &str,
        user_info: &UserInfo,
        amount: u64,
    ) -> Result<(), String> {
        let current_balance = self.get_balance(user_info, token);

        if current_balance.0 < amount {
            return Err(format!(
                "Insufficient balance: user {} has {} {} tokens, trying to remove {}",
                user_info.user, current_balance.0, token, amount
            ));
        }

        self.update_balances(
            token,
            vec![(user_info.get_key(), Balance(current_balance.0 - amount))],
        )
        .map_err(|e| e.to_string())
    }

    pub fn increment_nonce_and_save_user_info(
        &mut self,
        user_info: &UserInfo,
    ) -> Result<OrderbookEvent, String> {
        let mut updated_user_info = user_info.clone();
        updated_user_info.nonce = updated_user_info
            .nonce
            .checked_add(1)
            .ok_or("Nonce overflow")?;
        self.update_user_info_merkle_root(&updated_user_info)?;
        Ok(OrderbookEvent::NonceIncremented {
            user: user_info.user.clone(),
            nonce: updated_user_info.nonce,
        })
    }

    pub fn update_user_info_merkle_root(&mut self, user_info: &UserInfo) -> Result<(), String> {
        if user_info.nonce == 0 {
            return Err("User info nonce cannot be zero".to_string());
        }
        match &mut self.execution_state {
            ExecutionState::Full(state) => {
                state
                    .users_info_mt
                    .upsert(&user_info.get_key(), user_info.clone())
                    .map_err(|e| format!("Failed to update user info in monotree: {e}"))?;
                state
                    .light
                    .users_info
                    .insert(user_info.user.clone(), user_info.clone());
                self.users_info_merkle_root =
                    monotree_root_to_borshable(state.users_info_mt.root.as_ref());
            }
            ExecutionState::Light(state) => {
                state
                    .users_info
                    .insert(user_info.user.clone(), user_info.clone());
                state
                    .users_info
                    .entry(user_info.user.clone())
                    .or_insert_with(|| user_info.clone());
                self.users_info_merkle_root = H256::from([0u8; 32]);
            }
            ExecutionState::ZkVm(state) => {
                let key = user_info.get_key();
                let multi_proof = state
                    .users_info
                    .proof
                    .as_ref()
                    .ok_or_else(|| "Missing users_info multiproof".to_string())?;
                let key_bytes: [u8; 32] = (*key).into();
                let path = match multi_proof.proof_status(&key_bytes) {
                    Some(ProofStatus::Present(path)) => path,
                    Some(ProofStatus::Absent) | None => {
                        return Err(format!(
                            "No users_info proof found for user {} with key {}",
                            user_info.user,
                            hex::encode(key.as_slice())
                        ))
                    }
                };
                let leaf_hash = user_info.to_hash_bytes();
                let new_root = compute_root_from_proof(&leaf_hash, &path);
                self.users_info_merkle_root = H256::from(new_root);
            }
        }
        Ok(())
    }

    pub fn update_balances(
        &mut self,
        token: &str,
        balances_to_update: Vec<(H256, Balance)>,
    ) -> Result<(), String> {
        match &mut self.execution_state {
            ExecutionState::Full(state) => {
                let tree = state
                    .balances_mt
                    .entry(token.to_string())
                    .or_insert_with(MonotreeCommitment::default);
                tree.upsert_batch(balances_to_update.iter().cloned())
                    .map_err(|e| {
                        format!("Failed to update balances on token {token} in monotree: {e}")
                    })?;
                let light_balances = state.light.balances.entry(token.to_string()).or_default();
                for (user_info_key, balance) in &balances_to_update {
                    light_balances.insert(*user_info_key, balance.clone());
                }
                self.balances_merkle_roots.insert(
                    token.to_string(),
                    monotree_root_to_borshable(tree.root.as_ref()),
                );
            }
            ExecutionState::Light(state) => {
                let token_entry = state
                    .balances
                    .get_mut(token)
                    .ok_or_else(|| format!("Token {token} is not found in allowed tokens"))?;
                for (user_info_key, balance) in balances_to_update {
                    token_entry.insert(user_info_key, balance);
                }
                self.balances_merkle_roots
                    .entry(token.to_string())
                    .or_insert_with(|| H256::from([0u8; 32]));
            }
            ExecutionState::ZkVm(state) => {
                let witness = state.balances.get(token).ok_or_else(|| {
                    format!("No balance witness found for token {token} while running in ZkVm mode")
                })?;
                if balances_to_update.is_empty() {
                    return Ok(());
                }
                let mut current_root = self
                    .balances_merkle_roots
                    .get(token)
                    .map(|h| h.as_slice().try_into().unwrap())
                    .unwrap_or([0u8; 32]);

                let multi_proof = witness.proof.as_ref().ok_or_else(|| {
                    format!("Missing balance proof for token {token} while running in ZkVm mode")
                })?;

                for (user_info_key, balance) in balances_to_update.iter() {
                    let key_bytes: [u8; 32] = (*user_info_key).into();
                    let path = match multi_proof.proof_status(&key_bytes) {
                        Some(ProofStatus::Present(path)) => path,
                        Some(ProofStatus::Absent) | None => {
                            return Err(format!(
                                "Missing balance proof for token {token} and user key {}",
                                hex::encode(user_info_key.as_slice())
                            ));
                        }
                    };
                    let leaf_hash = balance.to_hash_bytes();
                    current_root = compute_root_from_proof(&leaf_hash, &path);
                }

                self.balances_merkle_roots
                    .insert(token.to_string(), H256::from(current_root));
            }
        }

        Ok(())
    }
}

impl borsh::BorshSerialize for FullState {
    fn serialize<W: std::io::Write>(
        &self,
        _writer: &mut W,
    ) -> std::result::Result<(), std::io::Error> {
        panic!("FullState::serialize: todo!()")
    }
}

impl borsh::BorshDeserialize for FullState {
    fn deserialize_reader<R: std::io::Read>(
        _reader: &mut R,
    ) -> std::result::Result<Self, std::io::Error> {
        panic!("FullState::deserialize: todo!()")
    }
}

type Sha256Monotree = Monotree<DefaultDatabase, Sha2>;

pub(crate) fn monotree_root_to_borshable(root: Option<&MonotreeHash>) -> H256 {
    match root {
        Some(hash) => H256::from(*hash),
        None => H256::from([0u8; 32]),
    }
}

pub struct MonotreeCommitment<T: MonotreeValue + Clone> {
    pub tree: Sha256Monotree,
    pub root: Option<MonotreeHash>,
    _marker: PhantomData<T>,
}

impl<T: MonotreeValue + Clone> Default for MonotreeCommitment<T> {
    fn default() -> Self {
        Self::new("monotree")
    }
}

impl<T: MonotreeValue + Clone> MonotreeCommitment<T> {
    pub fn new(namespace: &str) -> Self {
        Self {
            tree: Sha256Monotree::new(namespace),
            root: None,
            _marker: PhantomData,
        }
    }

    pub fn from_iter(
        namespace: &str,
        entries: impl IntoIterator<Item = (H256, T)>,
    ) -> monotree::Result<Self> {
        let mut commitment = Self::new(namespace);
        commitment.upsert_batch(entries)?;
        Ok(commitment)
    }

    pub fn default_from_iter(
        entries: impl IntoIterator<Item = (H256, T)>,
    ) -> monotree::Result<Self> {
        Self::from_iter("monotree", entries)
    }

    pub fn upsert(&mut self, key: &H256, value: T) -> monotree::Result<()> {
        let key_bytes: [u8; 32] = (*key).into();
        let leaf_hash: [u8; 32] = value.to_hash_bytes();
        self.root = self
            .tree
            .insert(self.root.as_ref(), &key_bytes, &leaf_hash)?;
        Ok(())
    }

    pub fn upsert_batch<I>(&mut self, entries: I) -> monotree::Result<()>
    where
        I: IntoIterator<Item = (H256, T)>,
    {
        let mut iter = entries.into_iter();

        let Some((first_key, first_value)) = iter.next() else {
            return Ok(());
        };

        self.tree.prepare();
        let mut current_root = self.root;

        let first_key_bytes: [u8; 32] = first_key.into();
        let first_leaf = first_value.to_hash_bytes();
        current_root = self
            .tree
            .insert(current_root.as_ref(), &first_key_bytes, &first_leaf)?;

        for (key, value) in iter {
            let key_bytes: [u8; 32] = key.into();
            let leaf_hash = value.to_hash_bytes();
            current_root = self
                .tree
                .insert(current_root.as_ref(), &key_bytes, &leaf_hash)?;
        }

        self.tree.commit();
        self.root = current_root;
        Ok(())
    }
}

impl<T: MonotreeValue + Clone> std::fmt::Debug for MonotreeCommitment<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonotreeCommitment")
            .field("root", &self.root.map(hex::encode))
            .finish()
    }
}

impl Clone for FullState {
    fn clone(&self) -> Self {
        let users_info_mt = MonotreeCommitment::default_from_iter(
            self.light
                .users_info
                .values()
                .map(|user| (user.get_key(), user.clone())),
        )
        .expect("Failed to rebuild users info monotree while cloning full state");

        let mut balances_mt = HashMap::new();
        for (token_name, balances) in &self.light.balances {
            let tree = MonotreeCommitment::default_from_iter(
                balances
                    .iter()
                    .map(|(key, balance)| (*key, balance.clone())),
            )
            .expect("Failed to rebuild balances monotree while cloning full state");
            balances_mt.insert(token_name.clone(), tree);
        }

        Self {
            light: self.light.clone(),
            users_info_mt,
            balances_mt,
        }
    }
}
