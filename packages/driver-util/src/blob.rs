use serde::{Deserialize, Serialize};

use crate::no_blobs;

type Hash = sha2::digest::Output<sha2::Sha256>;

/// Newtype for a hash that represents it's an object in the store.
#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Blob(Hash);

impl Blob {
    /// # Safety
    /// This function MUST only be used for constructing blobs from those saved to disk.
    pub unsafe fn from_hash(hash: Hash) -> Self {
        Self(hash)
    }
}

impl std::fmt::Display for Blob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Format as lowercase hex
        write!(f, "objects/{:x}", self.0)
    }
}

impl std::fmt::Debug for Blob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Format as lowercase hex, without the objects/ prefix
        write!(f, "{:x}", self.0)
    }
}

/// Trait that allows us to collect all blobs referenced by a value.
pub trait BlobTrace {
    fn trace(&self) -> impl Iterator<Item = &'_ Blob>;
}

impl BlobTrace for Blob {
    fn trace(&self) -> impl Iterator<Item = &'_ Blob> {
        std::iter::once(self)
    }
}

impl BlobTrace for crate::Error {
    fn trace(&self) -> impl Iterator<Item = &'_ Blob> {
        std::iter::empty()
    }
}

impl<T> BlobTrace for Option<T>
where
    T: BlobTrace,
{
    fn trace(&self) -> impl Iterator<Item = &'_ Blob> {
        self.iter().flat_map(|t| t.trace())
    }
}

impl<T, E> BlobTrace for Result<T, E>
where
    T: BlobTrace,
    E: BlobTrace,
{
    fn trace(&self) -> impl Iterator<Item = &'_ Blob> {
        match self {
            Ok(t) => Box::new(t.trace()) as Box<dyn Iterator<Item = &'_ Blob>>,
            Err(e) => Box::new(e.trace()) as Box<dyn Iterator<Item = &'_ Blob>>,
        }
    }
}

impl<T> BlobTrace for Vec<T>
where
    T: BlobTrace,
{
    fn trace(&self) -> impl Iterator<Item = &'_ Blob> {
        self.iter().flat_map(|t| t.trace())
    }
}

impl<K, V> BlobTrace for std::collections::BTreeMap<K, V>
where
    K: BlobTrace,
    V: BlobTrace,
{
    fn trace(&self) -> impl Iterator<Item = &'_ Blob> {
        self.iter().flat_map(|(k, v)| k.trace().chain(v.trace()))
    }
}

// TODO: expand this list as needed
no_blobs!(String);
no_blobs!(std::path::PathBuf);
