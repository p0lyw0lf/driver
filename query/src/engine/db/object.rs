use std::fmt::Display;

use boa_engine::{JsNativeError, JsResult, error::JsError};
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::{
    QueryContext,
    serde::SerializedMap,
    to_hash::{Hash, ToHash},
};

/// A store for all strings that would otherwise be too large to persist to disk multiple times.
/// Uniquely keyed by the hashes of the strings it stores.
#[derive(Default, Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Objects(SerializedMap<Object, Vec<u8>>);

/// Newtype for a hash that represents it's an object in the store.
#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct Object(Hash);

impl Object {
    /// SAFETY: this function MUST only be used for constructing objects from those saved to disk.
    pub unsafe fn from_hash(hash: Hash) -> Self {
        Self(hash)
    }

    pub fn contents_as_bytes(&self, ctx: &QueryContext) -> JsResult<Vec<u8>> {
        ctx.db().objects.with(self, |obj| {
            Ok(obj
                .ok_or_else(|| {
                    JsNativeError::typ().with_message(format!("object {} not found", self))
                })?
                .iter()
                .map(Clone::clone)
                .collect::<Vec<u8>>())
        })
    }

    pub fn contents_as_string(&self, ctx: &QueryContext) -> JsResult<String> {
        let bytes = self.contents_as_bytes(ctx)?;
        String::from_utf8(bytes).map_err(JsError::from_rust)
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
        let _ = self.0.insert_sync(object.clone(), contents);
    }

    pub fn with<T>(&self, object: &Object, f: impl Fn(Option<&[u8]>) -> T) -> T {
        let s = self.0.get_sync(object);
        match s {
            None => f(None),
            Some(s) => f(Some(&s.get()[..])),
        }
    }
}

impl Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Format as lowercase hex
        write!(f, "{:x}", self.0)
    }
}
