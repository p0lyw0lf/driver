use serde::{Deserialize, Serialize};

use crate::no_objects;

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

/// Trait that allows us to collect all objects present in a value.
pub trait ObjectTrace {
    fn trace(&self) -> impl Iterator<Item = &'_ Object>;
}

impl ObjectTrace for Object {
    fn trace(&self) -> impl Iterator<Item = &'_ Object> {
        std::iter::once(self)
    }
}

impl ObjectTrace for crate::Error {
    fn trace(&self) -> impl Iterator<Item = &'_ Object> {
        std::iter::empty()
    }
}

impl<T> ObjectTrace for Option<T>
where
    T: ObjectTrace,
{
    fn trace(&self) -> impl Iterator<Item = &'_ Object> {
        self.iter().flat_map(|t| t.trace())
    }
}

impl<T, E> ObjectTrace for Result<T, E>
where
    T: ObjectTrace,
    E: ObjectTrace,
{
    fn trace(&self) -> impl Iterator<Item = &'_ Object> {
        match self {
            Ok(t) => Box::new(t.trace()) as Box<dyn Iterator<Item = &'_ Object>>,
            Err(e) => Box::new(e.trace()) as Box<dyn Iterator<Item = &'_ Object>>,
        }
    }
}

impl<T> ObjectTrace for Vec<T>
where
    T: ObjectTrace,
{
    fn trace(&self) -> impl Iterator<Item = &'_ Object> {
        self.iter().flat_map(|t| t.trace())
    }
}

impl<K, V> ObjectTrace for std::collections::BTreeMap<K, V>
where
    K: ObjectTrace,
    V: ObjectTrace,
{
    fn trace(&self) -> impl Iterator<Item = &'_ Object> {
        self.iter().flat_map(|(k, v)| k.trace().chain(v.trace()))
    }
}

// TODO: expand this list as needed
no_objects!(());
no_objects!(String);
no_objects!(std::path::PathBuf);
