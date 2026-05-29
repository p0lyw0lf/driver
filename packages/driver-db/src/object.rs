use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::Digest as _;

use crate::Options;
use driver_util::SerializedMap;

/// A store for all strings/blobs that would otherwise be too large to persist to disk multiple
/// times. "Uniquely" keyed by the hashes of the strings/blobs it stores.
#[derive(Debug, Default, PartialEq)]
pub struct Objects {
    cache: SerializedMap<Object, Vec<u8>>,
}

type Hash = sha2::digest::Output<sha2::Sha256>;

/// Newtype for a hash that represents it's an object in the store.
#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Object(Hash);

impl Object {
    /// # Safety
    /// This function MUST only be used for constructing objects from those saved to disk.
    pub unsafe fn from_hash(hash: Hash) -> Self {
        Self(hash)
    }
}

impl Objects {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn store(&self, options: &Options, contents: Vec<u8>) -> driver_util::Result<Object> {
        let object = Object(sha2::Sha256::digest(&contents[..]));
        // SAFETY: we just calculated the hash
        unsafe { self.store_raw(options, object.clone(), contents)? };
        Ok(object)
    }

    /// # Safety
    /// `object` MUST be the hash of `contents`
    pub unsafe fn store_raw(
        &self,
        options: &Options,
        object: Object,
        contents: Vec<u8>,
    ) -> driver_util::Result<()> {
        // First, we need to write the contents to the specified file, if not already written.
        // We do this first so that we're never in a state where an entry exists but a file doesn't.
        let filename = self.object_filename(options, &object);
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
    pub fn load(&self, options: &Options, object: Object) -> driver_util::Result<Vec<u8>> {
        Ok(match self.cache.entry_sync(object.clone()) {
            scc::hash_map::Entry::Vacant(entry) => {
                let filename = self.object_filename(options, &object);
                let value = std::fs::read(&filename)?;
                let _ = entry.insert_entry(value.clone());
                value
            }
            scc::hash_map::Entry::Occupied(entry) => entry.get().clone(),
        })
    }

    /// This will create a hardlink from the file in the object store to the specified output path
    pub fn copy(
        &self,
        options: &Options,
        object: &Object,
        output_filename: &Path,
    ) -> driver_util::Result<()> {
        let input_filename = self.object_filename(options, object);
        if std::fs::exists(output_filename)? {
            std::fs::remove_file(output_filename)?;
        }
        std::fs::hard_link(&input_filename, output_filename)?;
        Ok(())
    }

    fn object_filename(&self, options: &Options, object: &Object) -> PathBuf {
        options.objects_path.join(format!("{:?}", object))
    }
}

impl std::fmt::Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Format as lowercase hex
        write!(f, "objects/{:x}", self.0)
    }
}

impl std::fmt::Debug for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Format as lowercase hex, without the objects/ prefix
        write!(f, "{:x}", self.0)
    }
}
