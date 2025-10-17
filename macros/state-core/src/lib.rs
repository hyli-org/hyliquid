use std::marker::PhantomData;

#[derive(Debug, Clone, Default)]
pub struct SMT<T> {
    _marker: PhantomData<T>,
}

#[derive(Debug, Clone, Default)]
pub struct ZkWitnessSet<T> {
    _marker: PhantomData<T>,
}

pub trait GetHashMapIndex<K> {
    fn hash_map_index(&self) -> &K;
}
