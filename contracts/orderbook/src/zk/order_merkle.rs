use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use borsh::{BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::BorshableMerkleProof;
use sparse_merkle_tree::traits::Value;

use crate::{
    model::{Order, OrderId, Pair},
    order_manager::OrderManager,
    zk::{Proof, ZkWitnessSet},
};

use super::{
    smt::{GetKey, SMT},
    H256,
};

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, BorshSerialize, BorshDeserialize, Default,
)]
pub struct OrderPriceLevel {
    pub pair: Pair,
    pub price: u64,
    pub order_ids: Vec<OrderId>,
}

impl OrderPriceLevel {
    pub fn from_queue(pair: &Pair, price: u64, queue: &VecDeque<OrderId>) -> Self {
        OrderPriceLevel {
            pair: pair.clone(),
            price,
            order_ids: queue.iter().cloned().collect(),
        }
    }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Default, Eq, PartialEq)]
pub struct OrderManagerRoots {
    pub orders_root: H256,
    pub bid_orders_root: H256,
    pub ask_orders_root: H256,
}

#[derive(Debug, Clone, Default, BorshDeserialize, BorshSerialize)]
pub struct OrderManagerWitnesses {
    pub orders: ZkWitnessSet<Order>,
    pub bid_orders: ZkWitnessSet<OrderPriceLevel>,
    pub ask_orders: ZkWitnessSet<OrderPriceLevel>,
    pub orders_owner: HashMap<OrderId, H256>,
}

#[derive(Debug)]
pub struct OrderManagerMerkles {
    pub orders: SMT<Order>,
    pub bid_orders: SMT<OrderPriceLevel>,
    pub ask_orders: SMT<OrderPriceLevel>,
}

impl OrderManagerMerkles {
    pub fn zero() -> Self {
        OrderManagerMerkles {
            orders: SMT::zero(),
            bid_orders: SMT::zero(),
            ask_orders: SMT::zero(),
        }
    }

    pub fn from_order_manager(manager: &OrderManager) -> Result<Self, String> {
        let mut orders_tree = SMT::zero();
        orders_tree
            .update_all_from_ref(manager.orders.values())
            .map_err(|e| format!("Failed to update orders SMT: {e}"))?;

        let mut bid_tree = SMT::zero();
        let bid_levels = collect_price_levels(&manager.bid_orders);
        if !bid_levels.is_empty() {
            bid_tree
                .update_all(bid_levels.into_iter())
                .map_err(|e| format!("Failed to update bid orders SMT: {e}"))?;
        }

        let mut ask_tree = SMT::zero();
        let ask_levels = collect_price_levels(&manager.ask_orders);
        if !ask_levels.is_empty() {
            ask_tree
                .update_all(ask_levels.into_iter())
                .map_err(|e| format!("Failed to update ask orders SMT: {e}"))?;
        }

        Ok(OrderManagerMerkles {
            orders: orders_tree,
            bid_orders: bid_tree,
            ask_orders: ask_tree,
        })
    }

    pub fn commitment(&self) -> OrderManagerRoots {
        OrderManagerRoots {
            orders_root: self.orders.root(),
            bid_orders_root: self.bid_orders.root(),
            ask_orders_root: self.ask_orders.root(),
        }
    }

    pub fn create_orders_witnesses(
        &self,
        orders: HashSet<Order>,
        bid_levels: HashSet<OrderPriceLevel>,
        ask_levels: HashSet<OrderPriceLevel>,
        orders_owner: HashMap<OrderId, H256>,
    ) -> Result<OrderManagerWitnesses, String> {
        let orders_witness =
            build_witness(&self.orders, orders, "orders merkle proof reconstruction")?;
        let bid_witness = build_witness(
            &self.bid_orders,
            bid_levels,
            "bid price levels merkle proof reconstruction",
        )?;
        let ask_witness = build_witness(
            &self.ask_orders,
            ask_levels,
            "ask price levels merkle proof reconstruction",
        )?;

        Ok(OrderManagerWitnesses {
            orders: orders_witness,
            bid_orders: bid_witness,
            ask_orders: ask_witness,
            orders_owner,
        })
    }
}

impl Default for OrderManagerMerkles {
    fn default() -> Self {
        OrderManagerMerkles::zero()
    }
}

impl OrderManagerWitnesses {
    pub fn commitment(&self) -> OrderManagerRoots {
        OrderManagerRoots {
            orders_root: self.orders.compute_root().expect("compute orders root"),
            bid_orders_root: self
                .bid_orders
                .compute_root()
                .expect("compute bid orders root"),
            ask_orders_root: self
                .ask_orders
                .compute_root()
                .expect("compute ask orders root"),
        }
    }

    pub fn into_order_manager(self) -> Result<OrderManager, String> {
        let mut manager = OrderManager::default();

        for order in &self.orders.values {
            manager.orders.insert(order.order_id.clone(), order.clone());
        }

        for level in &self.bid_orders.values {
            let entry = manager.bid_orders.entry(level.pair.clone()).or_default();
            entry.insert(level.price, VecDeque::from(level.order_ids.clone()));
        }

        for level in &self.ask_orders.values {
            let entry = manager.ask_orders.entry(level.pair.clone()).or_default();
            entry.insert(level.price, VecDeque::from(level.order_ids.clone()));
        }

        manager.orders_owner = self.orders_owner.clone();

        Ok(manager)
    }
}

pub fn collect_price_levels(
    side_map: &HashMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,
) -> HashSet<OrderPriceLevel> {
    let mut levels = HashSet::new();
    for (pair, price_map) in side_map {
        for (price, queue) in price_map {
            levels.insert(OrderPriceLevel::from_queue(pair, *price, queue));
        }
    }
    levels
}

fn build_witness<T>(
    tree: &SMT<T>,
    values: HashSet<T>,
    err_context: &str,
) -> Result<ZkWitnessSet<T>, String>
where
    T: std::fmt::Debug
        + BorshDeserialize
        + BorshSerialize
        + Value
        + GetKey
        + std::hash::Hash
        + Eq
        + Ord
        + Clone,
{
    if values.is_empty() {
        return Ok(ZkWitnessSet {
            values: HashSet::new(),
            proof: Proof::CurrentRootHash(tree.root()),
        });
    }

    let proof = tree
        .merkle_proof(values.iter())
        .map_err(|e| format!("Failed to create {err_context}: {e}"))?;

    let mut set = HashSet::new();
    for value in values.into_iter() {
        set.insert(value);
    }

    Ok(ZkWitnessSet {
        values: set,
        proof: Proof::Some(BorshableMerkleProof(proof)),
    })
}
