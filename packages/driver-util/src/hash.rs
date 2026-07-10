use std::ops::{Deref, DerefMut};

use sha2::Digest;

pub(crate) type Hash = sha2::digest::Output<sha2::Sha256>;

/// Helper struct that lets us shim types implementing [`std::hash::Hash`] into a sha256. "probably
/// fine" but I have no way to prove it...
pub(crate) struct Sha256Hasher {
    digest: sha2::Sha256,
}

impl std::hash::Hasher for Sha256Hasher {
    fn finish(&self) -> u64 {
        let hash = self.digest.clone().finalize();
        let bytes: &[u8; 32] = hash.as_ref();
        let low_bytes: &[u8; 8] = &bytes[0..8].try_into().unwrap();
        u64::from_le_bytes(*low_bytes)
    }

    fn write(&mut self, bytes: &[u8]) {
        self.digest.update(bytes);
    }
}

impl Deref for Sha256Hasher {
    type Target = sha2::Sha256;

    fn deref(&self) -> &Self::Target {
        &self.digest
    }
}

impl DerefMut for Sha256Hasher {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.digest
    }
}

impl From<sha2::Sha256> for Sha256Hasher {
    fn from(digest: sha2::Sha256) -> Self {
        Self { digest }
    }
}

impl Sha256Hasher {
    pub fn new() -> Self {
        sha2::Sha256::new().into()
    }

    pub fn finalize(self) -> Hash {
        self.digest.finalize()
    }

    fn update(&mut self, data: impl AsRef<[u8]>) {
        self.digest.update(data);
    }
}
