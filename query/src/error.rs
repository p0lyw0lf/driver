use std::fmt::Display;

use serde::{Deserialize, Serialize};
use sha2::Digest;

/// A very simple arbitrary error wrapper that just serializes everything to a String. Used in
/// place of anyhow so that we can clone it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Error(String);

impl Error {
    pub fn new(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl crate::to_hash::ToHash for Error {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"Error(");
        hasher.update(self.0.as_bytes());
        hasher.update(b")");
    }
}

impl<E> From<E> for Error
where
    E: std::error::Error,
{
    fn from(value: E) -> Self {
        Self(value.to_string())
    }
}

// Can't be done, though I think that's OK?
// impl std::error::Error for Error {}
