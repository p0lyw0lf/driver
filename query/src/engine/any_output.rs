use std::any::Any;
use std::any::TypeId;
use std::fmt::Debug;

use dyn_clone::DynClone;
use serde::{Deserialize, Serialize};

use crate::to_hash::ToHash;

/// NOTE: in order to make the lifetimes work out, we really really want it such that the output
/// is easily clone-able. This will eventually require string interning somewhere, not quite
/// sure where yet.
pub trait Output: ToHash + DynClone + Any + Debug + Send + Sync {}
dyn_clone::clone_trait_object!(Output);

/// NOTE: a newtype is needed to get around some associated type jank.
#[derive(Clone, Debug)]
pub struct AnyOutput(pub Box<dyn Output>);

impl ToHash for AnyOutput {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        // no prefix because we _do_ want this to be treated as the underlying value.
        self.0.run_hash(hasher);
    }
}
impl AnyOutput {
    pub fn new(t: impl Output) -> Self {
        if t.type_id() == TypeId::of::<AnyOutput>() {
            panic!("tried to put box inside of box");
        }
        Self(Box::new(t))
    }
    pub fn downcast<T: Output>(self) -> Option<Box<T>> {
        (self.0 as Box<dyn Any>).downcast().ok()
    }
}

impl PartialEq for AnyOutput {
    fn eq(&self, other: &Self) -> bool {
        self.to_hash() == other.to_hash()
    }
}

/// Macro to help generate Serialization/Deserializationn for the AnyOutput type. It is very janky
/// I can't just use typeid because erased-serde isn't compatible with postcard.
macro_rules! valid_outputs {
    ($($ty:ty,)*) => {
$(
    impl Output for $ty {}
)*
    impl Output for AnyOutput {}

static INDEX_TO_TYPE_ID: &[TypeId] = &[$(
    TypeId::of::<$ty>(),
)*];

impl Serialize for AnyOutput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use std::ops::Deref;
        use serde::ser::{Error, SerializeTuple};

        // This is stupid but I have so few types the O(n)-ness doesn't matter
        let want = <dyn Any>::type_id(self.0.deref());
        let _i = INDEX_TO_TYPE_ID.iter().position(|t| {
            &want == t
        }).ok_or_else(|| S::Error::custom("type not found"))?;

        let mut s = serializer.serialize_tuple(2)?;
        s.serialize_element(&_i)?;
        $(
            if _i == 0 {
                let v = <dyn Any>::downcast_ref::<$ty>(self.0.deref()).expect("TypeId compared equal but couldn't downcast");
                s.serialize_element(v)?;
                return s.end();
            }
            let _i = _i.saturating_sub(1);
        )*
        unreachable!()
    }
}

impl<'de> Deserialize<'de> for AnyOutput
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{Visitor, SeqAccess, Error};

        struct TupleVisitor;
        impl<'de> Visitor<'de> for TupleVisitor {
            type Value = AnyOutput;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(formatter, "AnyOutput")
            }

            #[inline]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>
            {
                let _i: usize = seq.next_element()?.ok_or_else(|| A::Error::custom("invalid length 0"))?;

                $(
                    if _i == 0 {
                        let v: $ty = seq.next_element()?.ok_or_else(|| A::Error::custom("invalid length 1"))?;
                        return Ok(AnyOutput::new(v));
                    }
                    let _i = _i.saturating_sub(1);
                )*
                Err(A::Error::custom("invalid tag"))
            }
        }

        deserializer.deserialize_tuple(2, TupleVisitor)
    }
}
    };
}

valid_outputs![
    crate::Result<crate::engine::db::Object>,
    crate::Result<crate::query::js::FileOutput>,
    crate::Result<Vec<std::path::PathBuf>>,
    crate::Result<crate::query::image::ImageObject>,
];
