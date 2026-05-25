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
    fn from(err: E) -> Self {
        let mut msg = err.to_string();
        // It seems many error implementations don't include source data as part of their message,
        // and instead we need to go down the stack manually
        let mut err = err.source();
        while let Some(e) = err {
            msg.push_str(&format!("\n\t{e}"));
            err = e.source();
        }
        Self(msg)
    }
}

// Can't be done, though I think that's OK?
// impl std::error::Error for Error {}
