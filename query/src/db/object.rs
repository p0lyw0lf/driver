use dashmap::DashMap;
use sha2::Digest;

use crate::to_hash::{Hash, ToHash};

/// A store for all strings that would otherwise be too large to persist to disk multiple times.
/// Uniquely keyed by the hashes of the strings it stores.
#[derive(Default, Debug)]
pub struct Objects(DashMap<Object, Vec<u8>>);

/// Newtype for a hash that represents it's an object in the store.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct Object(Hash);

impl ToHash for Object {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"Object(");
        hasher.update(self.0);
        hasher.update(b")");
    }
}

impl Objects {
    pub fn store(&self, s: Vec<u8>) -> Object {
        let object = Object(sha2::Sha256::digest(&s[..]));
        self.0.insert(object.clone(), s);
        object
    }

    pub fn get(&self, object: &Object) -> Option<impl AsRef<[u8]> + '_> {
        let s = self.0.get(object);
        // 1st map: map inside Option
        // 2nd map: create a "MappedRef" that implements the traits we want.
        s.map(|s| s.map(|s| s))
    }
}
