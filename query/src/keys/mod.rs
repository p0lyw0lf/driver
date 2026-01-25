use std::ops::Deref;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use sha2::Digest;

/// A sha256 digest is used as a key for _everything_ because it provides exact tracking of "thing
/// has changed" with effectively zero false negatives.
pub type Hash = sha2::digest::Output<sha2::Sha256>;

pub trait ToHash: std::any::Any {
    fn to_hash(&self) -> Hash;
}

impl ToHash for Hash {
    fn to_hash(&self) -> Hash {
        *self
    }
}

/// NOTE: a newtype is needed to get around some associated type jank.
pub struct AnyOutput(pub Box<dyn ToHash>);

impl ToHash for AnyOutput {
    fn to_hash(&self) -> Hash {
        self.0.to_hash()
    }
}

impl AnyOutput {
    pub fn new<T: ToHash + 'static>(t: T) -> Self {
        Self(Box::new(t))
    }
}

macro_rules! wrapper {
    ($wrapper:ident) => {
        impl<A> ToHash for $wrapper<A>
        where
            A: ToHash,
        {
            fn to_hash(&self) -> Hash {
                self.deref().to_hash()
            }
        }
    };
}

wrapper!(Box);
wrapper!(Rc);
wrapper!(Arc);

macro_rules! tuple {
    ($($ty:ident),*) => {
        impl<$($ty,)*> ToHash for ($($ty,)*) where
            $($ty: ToHash,)*
        {
            fn to_hash(&self) -> Hash {
                #[allow(non_snake_case)]
                let ($($ty,)*) = self;
                let mut hasher = sha2::Sha256::new();
                $(
                hasher.update($ty.to_hash());
                )*
                hasher.finalize()
            }
        }
    }
}

impl<A> ToHash for (A,)
where
    A: ToHash,
{
    fn to_hash(&self) -> Hash {
        self.0.to_hash()
    }
}
tuple!(A, B);
tuple!(A, B, C);
tuple!(A, B, C, D);
tuple!(A, B, C, D, E);

impl<A> ToHash for Vec<A>
where
    A: ToHash,
{
    fn to_hash(&self) -> Hash {
        let mut hasher = sha2::Sha256::new();
        for a in self.iter() {
            hasher.update(a.to_hash());
        }
        hasher.finalize()
    }
}

impl ToHash for String {
    fn to_hash(&self) -> Hash {
        let mut hasher = sha2::Sha256::new();
        hasher.update(self.as_bytes());
        hasher.finalize()
    }
}

impl ToHash for PathBuf {
    fn to_hash(&self) -> Hash {
        let mut hasher = sha2::Sha256::new();
        hasher.update(self.as_os_str().as_encoded_bytes());
        hasher.finalize()
    }
}
