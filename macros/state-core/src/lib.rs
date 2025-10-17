#[derive(Debug, Clone)]
pub struct SMT<T> {
    inner: T,
}

impl<T: Default> Default for SMT<T> {
    fn default() -> Self {
        Self {
            inner: T::default(),
        }
    }
}

impl<T> SMT<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    pub fn set_inner(&mut self, inner: T) {
        self.inner = inner;
    }
}

#[derive(Debug, Clone)]
pub struct ZkWitnessSet<T> {
    inner: T,
}

impl<T: Default> Default for ZkWitnessSet<T> {
    fn default() -> Self {
        Self {
            inner: T::default(),
        }
    }
}

impl<T> ZkWitnessSet<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn from(inner: T) -> Self {
        Self::new(inner)
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn take_inner(&mut self) -> T
    where
        T: Default,
    {
        std::mem::take(&mut self.inner)
    }

    pub fn set_inner(&mut self, inner: T) {
        self.inner = inner;
    }
}

pub trait GetHashMapIndex<K> {
    fn hash_map_index(&self) -> &K;
}
