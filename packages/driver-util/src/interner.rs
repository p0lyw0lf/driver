use std::marker::PhantomData;

use scc::HashMap;
use serde::{Deserialize, Serialize};

use crate::{Hash, ToHash};

/// Datastructure used for interning keys into hashes, so that we can:
/// 1. Not have to pass around & clone potentially very large keys if we don't need the underlying data
/// 2. Have one canonical source for keys if we do need to read from them.
#[derive(Debug, Default)]
pub struct Interner<Key> {
    map: HashMap<HashInterned<Key>, Key>,
}

/// A key associated with a specific [`Interner`]. Users SHOULD have just one [`Interner`] in their
/// program, so that they can't confuse which [`Interner`] a [`HashInterned`] belongs to.
#[derive(Serialize, Deserialize)]
pub struct HashInterned<Key>(Hash, PhantomData<Key>);

/// Default impl takes a bound on `Key`, which we don't want, so we have to write these by hand
/// unfortunately...
impl<Key> std::fmt::Debug for HashInterned<Key> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

impl<Key> Clone for HashInterned<Key> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<Key> Copy for HashInterned<Key> {}

impl<Key> PartialEq for HashInterned<Key> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<Key> Eq for HashInterned<Key> {}

impl<Key> PartialOrd for HashInterned<Key> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<Key> Ord for HashInterned<Key> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl<Key> std::hash::Hash for HashInterned<Key> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<Key: std::hash::Hash> Interner<Key> {
    pub fn insert(&self, key: Key) -> HashInterned<Key> {
        let hash = HashInterned(key.to_hash(), PhantomData);
        let _ = self.map.upsert_sync(hash, key);
        hash
    }

    pub fn with<T>(&self, hash: &HashInterned<Key>, f: impl FnOnce(Option<&Key>) -> T) -> T {
        let entry = self.map.get_sync(hash);
        match entry {
            Some(entry) => f(Some(entry.get())),
            None => f(None),
        }
    }
}
