use std::collections::HashMap;
use std::mem;

#[derive(Debug, Clone, Default)]
pub struct SMT<T> {
    inner: HashMap<String, T>,
}

impl<T> SMT<T> {
    pub fn from_map(map: HashMap<String, T>) -> Self {
        Self { inner: map }
    }

    pub fn into_map(self) -> HashMap<String, T> {
        self.inner
    }

    pub fn take_inner(&mut self) -> HashMap<String, T> {
        mem::take(&mut self.inner)
    }

    pub fn inner(&self) -> &HashMap<String, T> {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut HashMap<String, T> {
        &mut self.inner
    }
}

#[derive(Debug, Clone, Default)]
pub struct ZkWitnessSet<T> {
    inner: HashMap<String, T>,
}

impl<T> ZkWitnessSet<T> {
    pub fn from_map(map: HashMap<String, T>) -> Self {
        Self { inner: map }
    }

    pub fn take_inner(&mut self) -> HashMap<String, T> {
        mem::take(&mut self.inner)
    }

    pub fn inner(&self) -> &HashMap<String, T> {
        &self.inner
    }

    pub fn set_inner(&mut self, map: HashMap<String, T>) {
        self.inner = map;
    }
}

pub trait GetHashMapIndex<K> {
    fn hash_map_index(&self) -> &K;
}
