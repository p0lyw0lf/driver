use std::ops::Deref;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use sha2::Digest;

/// A sha256 digest is used as a key for _everything_ because it provides exact tracking of "thing
/// has changed" with effectively zero false negatives.
pub type Hash = sha2::digest::Output<sha2::Sha256>;

/// This trait is needed for dyn-compatibility, otherwise we aren't able to make a type that is
/// `dyn Hash + Any`.
pub trait ToHash {
    fn run_hash(&self, hasher: &mut sha2::Sha256);
    fn to_hash(&self) -> Hash {
        let mut hasher = sha2::Sha256::new();
        self.run_hash(&mut hasher);
        hasher.finalize()
    }
}

impl ToHash for Hash {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"Hash(");
        hasher.update(self);
        hasher.update(b")");
    }
}

macro_rules! wrapper {
    ($wrapper:ident) => {
        impl<A> ToHash for $wrapper<A>
        where
            A: ToHash,
        {
            fn run_hash(&self, hasher: &mut sha2::Sha256) {
                hasher.update(stringify!($wrapper).as_bytes());
                self.deref().run_hash(hasher);
            }
        }
    };
}

wrapper!(Box);
wrapper!(Rc);
wrapper!(Arc);

impl<A> ToHash for Vec<A>
where
    A: ToHash,
{
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"Vec[");
        for a in self.iter() {
            a.run_hash(hasher);
        }
        hasher.update(b"]");
    }
}

impl<T, E> ToHash for Result<T, E>
where
    T: ToHash,
    E: ToHash,
{
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        match self {
            Ok(t) => {
                hasher.update(b"Result::Ok(");
                t.run_hash(hasher);
                hasher.update(b")");
            }
            Err(e) => {
                hasher.update(b"Result::Err(");
                e.run_hash(hasher);
                hasher.update(b")");
            }
        }
    }
}

impl ToHash for String {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"String(");
        hasher.update(self.as_bytes());
        hasher.update(b")");
    }
}

impl ToHash for PathBuf {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"PathBuf(");
        hasher.update(self.as_os_str().as_encoded_bytes());
        hasher.update(b")");
    }
}

impl ToHash for () {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"()");
    }
}
