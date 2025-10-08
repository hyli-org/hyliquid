use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::auth::UserAuth;

/// Shared state accessible across all Goose tasks
#[derive(Clone)]
pub struct SharedState {
    pub rng: Arc<Mutex<ChaCha8Rng>>,
    pub order_tracker: Arc<Mutex<OrderTracker>>,
    pub mid_price: Arc<Mutex<MidPrice>>,
}

impl SharedState {
    pub fn new(seed: u64, initial_mid: u64) -> Self {
        let rng = if seed == 0 {
            ChaCha8Rng::from_entropy()
        } else {
            ChaCha8Rng::seed_from_u64(seed)
        };

        SharedState {
            rng: Arc::new(Mutex::new(rng)),
            order_tracker: Arc::new(Mutex::new(OrderTracker::new())),
            mid_price: Arc::new(Mutex::new(MidPrice::new(initial_mid))),
        }
    }

    /// Get a random number in range [min, max] (inclusive)
    pub fn random_range(&self, min: u64, max: u64) -> u64 {
        if min >= max {
            return min;
        }
        let mut rng = self.rng.lock().unwrap();
        rng.gen_range(min..=max)
    }

    /// Get a random boolean with given probability (0.0 to 1.0)
    pub fn random_bool(&self, probability: f64) -> bool {
        let mut rng = self.rng.lock().unwrap();
        rng.gen_bool(probability)
    }

    /// Get a random drift for mid price
    pub fn random_drift(&self, max_drift: i64) -> i64 {
        if max_drift == 0 {
            return 0;
        }
        let mut rng = self.rng.lock().unwrap();
        rng.gen_range(-max_drift..=max_drift)
    }
}

/// Tracks orders created during the test
pub struct OrderTracker {
    orders: VecDeque<OrderInfo>,
    max_size: usize,
}

#[derive(Clone, Debug)]
pub struct OrderInfo {
    pub order_id: String,
}

impl OrderTracker {
    pub fn new() -> Self {
        OrderTracker {
            orders: VecDeque::new(),
            max_size: 100, // Will be overridden by config
        }
    }

    pub fn with_max_size(max_size: usize) -> Self {
        OrderTracker {
            orders: VecDeque::new(),
            max_size,
        }
    }

    /// Add a new order to tracking
    pub fn add_order(&mut self, order_id: String) {
        let info = OrderInfo { order_id };

        self.orders.push_back(info);

        // Keep only the most recent orders
        while self.orders.len() > self.max_size {
            self.orders.pop_front();
        }
    }

    /// Get old orders to cancel (oldest X%)
    pub fn get_orders_to_cancel(&mut self, percentage: u32) -> Vec<OrderInfo> {
        if self.orders.is_empty() {
            return Vec::new();
        }

        let count = (self.orders.len() * percentage as usize / 100).max(1);
        let mut result = Vec::new();

        for _ in 0..count.min(self.orders.len()) {
            if let Some(order) = self.orders.pop_front() {
                result.push(order);
            }
        }

        result
    }

    /// Get count of tracked orders
    pub fn count(&self) -> usize {
        self.orders.len()
    }
}

/// Manages the dynamic mid price with random walk
pub struct MidPrice {
    current: u64,
    initial: u64,
}

impl MidPrice {
    pub fn new(initial: u64) -> Self {
        MidPrice {
            current: initial,
            initial,
        }
    }

    /// Get current mid price
    pub fn get(&self) -> u64 {
        self.current
    }

    /// Apply a drift to the mid price
    pub fn apply_drift(&mut self, drift: i64) {
        if drift < 0 {
            let abs_drift = drift.unsigned_abs();
            self.current = self.current.saturating_sub(abs_drift);
        } else {
            self.current = self.current.saturating_add(drift as u64);
        }

        // Keep within reasonable bounds (10% to 1000% of initial)
        let min_price = self.initial / 10;
        let max_price = self.initial * 10;
        self.current = self.current.clamp(min_price, max_price);
    }
}

/// Per-user state stored in Goose user session
pub struct UserState {
    pub auth: UserAuth,
    pub nonce: u32,
    pub session_key_added: bool,
    pub base_balance: u64,
    pub quote_balance: u64,
}

impl UserState {
    pub fn new(user_id: usize) -> anyhow::Result<Self> {
        let identity = format!("loadtest_user_{user_id}");
        let auth = UserAuth::new(&identity)?;

        Ok(UserState {
            auth,
            nonce: 0,
            session_key_added: false,
            base_balance: 0,
            quote_balance: 0,
        })
    }

    /// Increment and return the current nonce
    pub fn next_nonce(&mut self) -> u32 {
        let current = self.nonce;
        self.nonce += 1;
        current
    }

    /// Generate a unique order ID
    pub fn generate_order_id(&self, prefix: &str) -> String {
        format!(
            "{}_{}_{}_{}",
            prefix,
            self.auth.identity,
            self.nonce,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_state_creation() {
        let state = SharedState::new(42, 1000);
        let val1 = state.random_range(1, 100);
        let val2 = state.random_range(1, 100);
        assert!((1..=100).contains(&val1));
        assert!((1..=100).contains(&val2));
    }

    #[test]
    fn test_order_tracker() {
        let mut tracker = OrderTracker::with_max_size(3);

        tracker.add_order("order1".to_string());
        tracker.add_order("order2".to_string());
        tracker.add_order("order3".to_string());

        assert_eq!(tracker.count(), 3);

        // Adding a 4th should evict the first
        tracker.add_order("order4".to_string());
        assert_eq!(tracker.count(), 3);
    }

    #[test]
    fn test_mid_price_drift() {
        let mut mid = MidPrice::new(1000);
        assert_eq!(mid.get(), 1000);

        mid.apply_drift(50);
        assert_eq!(mid.get(), 1050);

        mid.apply_drift(-30);
        assert_eq!(mid.get(), 1020);
    }

    #[test]
    fn test_user_state() {
        let mut user = UserState::new(1).unwrap();
        assert_eq!(user.auth.identity, "loadtest_user_1");

        let nonce1 = user.next_nonce();
        let nonce2 = user.next_nonce();
        assert_eq!(nonce1, 0);
        assert_eq!(nonce2, 1);
    }
}
