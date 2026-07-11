use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

use driver_util::{Hash, ToHash};

/// A key associated with a specific [`Interner`]. Users SHOULD have just one [`Interner`] in their
/// program, so that they can't confuse which [`Interner`] a [`Hashed`] belongs to.
#[derive(Serialize, Deserialize)]
pub struct Hashed<Key>(Hash, PhantomData<Key>);

/// Default impl takes a bound on `Key`, which we don't want, so we have to write these by hand
/// unfortunately...
impl<Key> std::fmt::Debug for Hashed<Key> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

impl<Key> Clone for Hashed<Key> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<Key> Copy for Hashed<Key> {}

impl<Key> PartialEq for Hashed<Key> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<Key> Eq for Hashed<Key> {}

impl<Key> PartialOrd for Hashed<Key> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<Key> Ord for Hashed<Key> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl<Key> std::hash::Hash for Hashed<Key> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<Key: std::hash::Hash> Hashed<Key> {
    pub(crate) fn new(key: &Key) -> Self {
        Self(key.to_hash(), PhantomData)
    }
}
