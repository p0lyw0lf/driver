use std::collections::BTreeMap;
use std::fmt::Debug;
use std::hash::Hash;

use serde::{Deserialize, Serialize};
use wasmtime::component::Resource;

use driver_util::Object;

pub trait ValueTerm {
    type R: Hash + Eq + Ord + Debug + Clone + Serialize + for<'de> Deserialize<'de>;
}
pub type Ref<T> = <T as ValueTerm>::R;

/// All the simple JSON-style values that can be serialized/deserialized losslessly
#[derive(Default, Hash, PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Serialize, Deserialize)]
pub enum Value<T>
where
    T: ValueTerm,
{
    #[default]
    Null,
    Bool(bool),
    Int(i32),
    String(String),
    Array(Vec<Ref<T>>),
    Object(BTreeMap<String, Ref<T>>),
    Blob(Object),
}

#[derive(Default, Hash, PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Serialize, Deserialize)]
pub struct Inline;
impl ValueTerm for Inline {
    type R = Value<Inline>;
}

pub type InlineValue = Value<Inline>;

impl std::fmt::Display for InlineValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InlineValue::Null => f.write_str("null"),
            InlineValue::Bool(b) => f.write_str(if *b { "true" } else { "false" }),
            InlineValue::Int(i) => std::fmt::Display::fmt(i, f),
            InlineValue::String(s) => write!(f, "\"{}\"", s),
            InlineValue::Array(vs) => {
                f.write_str("[")?;
                for (i, v) in vs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    std::fmt::Display::fmt(v, f)?;
                }
                f.write_str("]")?;
                Ok(())
            }
            InlineValue::Object(btree_map) => {
                f.write_str("{")?;
                for (i, (k, v)) in btree_map.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "\"{}\": {}", k, v)?;
                }
                f.write_str("}")?;
                Ok(())
            }
            InlineValue::Blob(blob) => std::fmt::Display::fmt(&blob, f),
        }
    }
}

impl driver_util::ObjectTrace for InlineValue {
    fn trace(&self) -> impl Iterator<Item = &'_ driver_util::Object> {
        fn mk_box<'a, T: 'a>(i: impl Iterator<Item = T> + 'a) -> Box<dyn Iterator<Item = T> + 'a> {
            Box::new(i) as Box<dyn Iterator<Item = T> + 'a>
        }

        match self {
            InlineValue::Null => mk_box(std::iter::empty()),
            InlineValue::Bool(_) => mk_box(std::iter::empty()),
            InlineValue::Int(_) => mk_box(std::iter::empty()),
            InlineValue::String(_) => mk_box(std::iter::empty()),
            InlineValue::Array(js_values) => mk_box(js_values.trace()),
            InlineValue::Object(btree_map) => mk_box(btree_map.trace()),
            InlineValue::Blob(blob) => mk_box(blob.trace()),
        }
    }
}

/// Newtype so we can get the necessary trait implementations to make the Trees That Grow types
/// happy
#[derive(Debug)]
pub struct OutlineRef(Resource<super::host::Value>);

impl From<OutlineRef> for Resource<super::host::Value> {
    fn from(value: OutlineRef) -> Self {
        value.0
    }
}

impl From<Resource<super::host::Value>> for OutlineRef {
    fn from(value: Resource<super::host::Value>) -> Self {
        Self(value)
    }
}

impl PartialEq for OutlineRef {
    fn eq(&self, _other: &Self) -> bool {
        panic!("MUST NOT compare outline values")
    }
}

impl Eq for OutlineRef {}

impl PartialOrd for OutlineRef {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OutlineRef {
    fn cmp(&self, _other: &Self) -> std::cmp::Ordering {
        panic!("MUST NOT compare outline values")
    }
}

impl Clone for OutlineRef {
    fn clone(&self) -> Self {
        Self(Resource::new_borrow(self.0.rep()))
    }
}

impl Hash for OutlineRef {
    fn hash<H: std::hash::Hasher>(&self, _state: &mut H) {
        panic!("MUST NOT hash outline values")
    }
}

impl Serialize for OutlineRef {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        panic!("MUST NOT serialize outline values")
    }
}

impl<'de> Deserialize<'de> for OutlineRef {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        panic!("MUST NOT deseralize outline values")
    }
}

#[derive(Default, Hash, PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Serialize, Deserialize)]
pub struct Outline;
impl ValueTerm for Outline {
    type R = OutlineRef;
}

pub type OutlineValue = Value<Outline>;
