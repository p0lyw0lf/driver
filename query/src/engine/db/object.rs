use std::{
    fmt::Display,
    path::{Path, PathBuf},
};

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
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Objects {
    cache: SerializedMap<Object, Vec<u8>>,
    base_dir: PathBuf,
}

/// Newtype for a hash that represents it's an object in the store.
#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct Object(Hash);

impl Object {
    /// SAFETY: this function MUST only be used for constructing objects from those saved to disk.
    pub unsafe fn from_hash(hash: Hash) -> Self {
        Self(hash)
    }

    pub fn contents_as_bytes(&self, ctx: &QueryContext) -> JsResult<Vec<u8>> {
        let obj = ctx.db().objects.load(self.clone()).map_err(|err| {
            JsNativeError::typ().with_message(format!("loading object {}: {}", self, err))
        })?;
        Ok(obj)
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
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            cache: SerializedMap::default(),
            base_dir,
        }
    }

    pub fn store(&self, contents: Vec<u8>) -> crate::Result<Object> {
        let object = Object(sha2::Sha256::digest(&contents[..]));
        // SAFETY: we just calculated the hash
        unsafe { self.store_raw(object.clone(), contents)? };
        Ok(object)
    }

    /// SAFETY: object must be the hash of contents
    pub unsafe fn store_raw(&self, object: Object, contents: Vec<u8>) -> crate::Result<()> {
        // First, we need to write the contents to the specified file, if not already written.
        // We do this first so that we're never in a state where an entry exists but a file doesn't.
        let filename = self.object_filename(&object);
        if !std::fs::exists(&filename)? {
            // TODO: should we use async_fs here, or is our existing threadpool enough?
            // Right now I don't want to color all the functions, so let's hope the threadpool is
            // enough lol.
            std::fs::write(&filename, &contents)?;
        }

        // Then, we insert the file
        let _ = self.cache.insert_sync(object.clone(), contents);
        Ok(())
    }

    /// This will return an error if the file doesn't exist, because the only way we should have
    /// access to objects is by having created a file beforehand.
    pub fn load(&self, object: Object) -> crate::Result<Vec<u8>> {
        Ok(match self.cache.entry_sync(object.clone()) {
            scc::hash_map::Entry::Vacant(entry) => {
                let filename = self.object_filename(&object);
                let value = std::fs::read(&filename)?;
                let _ = entry.insert_entry(value.clone());
                value
            }
            scc::hash_map::Entry::Occupied(entry) => entry.get().clone(),
        })
    }

    /// This will create a hardlink from the file in the object store to the specified output path
    pub async fn copy(&self, object: &Object, output_filename: &Path) -> crate::Result<()> {
        let input_filename = self.object_filename(object);
        if std::fs::exists(output_filename)? {
            async_fs::remove_file(output_filename).await?;
        }
        async_fs::hard_link(&input_filename, output_filename).await?;
        Ok(())
    }

    fn object_filename(&self, object: &Object) -> PathBuf {
        self.base_dir.join(object.to_string())
    }
}

impl Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Format as lowercase hex
        write!(f, "{:x}", self.0)
    }
}
