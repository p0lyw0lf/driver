use std::fmt::Display;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::to_hash::{Hash, ToHash};

/// A store for all strings that would otherwise be too large to persist to disk multiple times.
/// Uniquely keyed by the hashes of the strings it stores.
#[derive(Default, Debug)]
pub struct Objects(DashMap<Object, Vec<u8>>);

/// Newtype for a hash that represents it's an object in the store.
#[derive(Clone, Hash, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Object(Hash);

impl Object {
    /// SAFETY: this function MUST only be used for constructing objects from those saved to disk.
    pub unsafe fn from_hash(hash: Hash) -> Self {
        Self(hash)
    }
}

impl ToHash for Object {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"Object(");
        hasher.update(self.0);
        hasher.update(b")");
    }
}

impl Objects {
    pub fn store(&self, contents: Vec<u8>) -> Object {
        let object = Object(sha2::Sha256::digest(&contents[..]));
        // SAFETY: we just calculated the hash
        unsafe { self.store_raw(object.clone(), contents) };
        object
    }

    /// SAFETY: object must be the hash of contents
    pub unsafe fn store_raw(&self, object: Object, contents: Vec<u8>) {
        self.0.insert(object.clone(), contents);
    }

    pub fn get(&self, object: &Object) -> Option<impl AsRef<[u8]> + '_> {
        let s = self.0.get(object);
        // 1st map: map inside Option
        // 2nd map: create a "MappedRef" that implements the traits we want.
        s.map(|s| s.map(|s| s))
    }

    pub fn for_each<E>(&self, f: impl Fn(&Object, &Vec<u8>) -> Result<(), E>) -> Result<(), E> {
        for e in self.0.iter() {
            f(e.key(), e.value())?
        }
        Ok(())
    }
}

impl Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Format as lowercase hex
        write!(f, "{:x}", self.0)
    }
}
