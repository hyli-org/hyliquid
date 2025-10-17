#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtCommitment<T> {
    refreshed: bool,
    _marker: std::marker::PhantomData<T>,
}

impl<T> Default for SmtCommitment<T> {
    fn default() -> Self {
        Self {
            refreshed: false,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T> SmtCommitment<T> {
    pub fn refreshed() -> Self {
        Self {
            refreshed: true,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn is_refreshed(&self) -> bool {
        self.refreshed
    }

    pub fn mark_refreshed(&mut self) {
        self.refreshed = true;
    }
}

pub trait GetHashMapIndex<K> {
    fn hash_map_index(&self) -> &K;
}

pub struct RequiresIndex<V, K>(std::marker::PhantomData<(V, K)>)
where
    V: GetHashMapIndex<K>;

impl<V, K> RequiresIndex<V, K>
where
    V: GetHashMapIndex<K>,
{
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

#[derive(Debug, Clone)]
pub struct SmtWitness<T> {
    _marker: std::marker::PhantomData<T>,
}

impl<T> Default for SmtWitness<T> {
    fn default() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}
