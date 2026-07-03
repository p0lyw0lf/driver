use std::path::{Path, PathBuf};

use sha2::Digest as _;

use crate::Options;
use driver_util::{Blob, SerializedMap};

/// A store for all strings/blobs that would otherwise be too large to persist to disk multiple
/// times. "Uniquely" keyed by the hashes of the strings/blobs it stores.
#[derive(Debug, Default, PartialEq)]
pub struct Blobs {
    cache: SerializedMap<Blob, Vec<u8>>,
}

impl Blobs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn store(&self, options: &Options, contents: Vec<u8>) -> driver_util::Result<Blob> {
        let hash = sha2::Sha256::digest(&contents[..]);
        // SAFETY: we just calculated the hash
        let blob = unsafe { Blob::from_hash(hash) };
        // SAFETY: we just calculated the hash
        unsafe { self.store_raw(options, blob.clone(), contents)? };
        Ok(blob)
    }

    /// # Safety
    /// `blob` MUST be the hash of `contents`
    pub unsafe fn store_raw(
        &self,
        options: &Options,
        blob: Blob,
        contents: Vec<u8>,
    ) -> driver_util::Result<()> {
        // First, we need to write the contents to the specified file, if not already written.
        // We do this first so that we're never in a state where an entry exists but a file doesn't.
        let filename = self.blob_filename(options, &blob);
        if !std::fs::exists(&filename)? {
            // TODO: should we use async_fs here, or is our existing threadpool enough?
            // Right now I don't want to color all the functions, so let's hope the threadpool is
            // enough lol.
            std::fs::write(&filename, &contents)?;
        }

        // Then, we insert the file
        let _ = self.cache.insert_sync(blob.clone(), contents);
        Ok(())
    }

    /// This will return an error if the file doesn't exist, because the only way we should have
    /// access to blobs is by having created a file beforehand.
    pub fn load(&self, options: &Options, blob: Blob) -> driver_util::Result<Vec<u8>> {
        Ok(match self.cache.entry_sync(blob.clone()) {
            scc::hash_map::Entry::Vacant(entry) => {
                let filename = self.blob_filename(options, &blob);
                let value = std::fs::read(&filename)?;
                let _ = entry.insert_entry(value.clone());
                value
            }
            scc::hash_map::Entry::Occupied(entry) => entry.get().clone(),
        })
    }

    pub fn load_mmap(&self, options: &Options, blob: &Blob) -> driver_util::Result<memmap2::Mmap> {
        let filename = self.blob_filename(options, blob);
        let file = std::fs::File::open(&filename)?;
        // SAFETY: We don't do anything crazy with these files, if the user does then that's their
        // problem.
        // Ok but this is _actually_ unsafe tho, like "attacker can cause bad UB that crashes this
        // process" unsafe prolly, so I _shouldn't_ just rely on this, but meh I am laze :(
        let mmap = unsafe { memmap2::Mmap::map(&file) }?;
        Ok(mmap)
    }

    /// This will create a hardlink from the file in the blob store to the specified output path
    pub fn copy(
        &self,
        options: &Options,
        blob: &Blob,
        output_filename: &Path,
    ) -> driver_util::Result<()> {
        let input_filename = self.blob_filename(options, blob);
        if std::fs::exists(output_filename)? {
            std::fs::remove_file(output_filename)?;
        }
        std::fs::hard_link(&input_filename, output_filename)?;
        Ok(())
    }

    fn blob_filename(&self, options: &Options, blob: &Blob) -> PathBuf {
        options.blobs_path.join(format!("{:?}", blob))
    }

    /// MUST be called with the equivalent of an exclusive reference. Sorry the types don't work
    /// out...
    /// TODO: make this async probably? For faster deletion.
    pub(crate) fn retain(
        &self,
        options: &Options,
        f: impl Fn(&Blob) -> bool,
    ) -> driver_util::Result<()> {
        self.cache.clear_sync();

        // Read from the filesystem to get a list of all possible blobs
        for file in std::fs::read_dir(&options.blobs_path)? {
            let file = file?;
            let path = file.path();
            let hash = path.file_name().expect("blob didn't have filename?");
            let hash: [u8; 32] = hex::FromHex::from_hex(hash.as_encoded_bytes())?;
            // SAFETY: Object was read from filesystem
            let blob = unsafe { Blob::from_hash(hash.into()) };

            if !f(&blob) {
                // Delete the blob
                std::fs::remove_file(path)?;
            }
        }

        Ok(())
    }
}
