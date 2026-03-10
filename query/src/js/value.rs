use boa_engine::value::TryIntoJs;
use boa_engine::{Context, JsResult, value::TryFromJs};
use boa_engine::{JsError, JsNativeError};
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::js::JsObject;
use crate::to_hash::ToHash;

/// All the simple javascript values that can be serialized/deserialized losslessly
#[derive(Default, Hash, PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Serialize, Deserialize)]
pub enum JsValue {
    #[default]
    Undefined,
    Null,
    Bool(bool),
    Int(i32),
    String(String),
    Array(Vec<JsValue>),
    Store(JsObject),
}

impl TryFromJs for JsValue {
    fn try_from_js(value: &boa_engine::JsValue, context: &mut Context) -> JsResult<Self> {
        match value.variant() {
            boa_engine::JsVariant::Null => Ok(Self::Null),
            boa_engine::JsVariant::Undefined => Ok(Self::Undefined),
            boa_engine::JsVariant::Boolean(b) => Ok(Self::Bool(b)),
            boa_engine::JsVariant::String(js_string) => Ok(Self::String(
                js_string.to_std_string().map_err(JsError::from_rust)?,
            )),
            boa_engine::JsVariant::Float64(_) => Err(JsNativeError::typ()
                .with_message("cannot serialize float")
                .into()),
            boa_engine::JsVariant::Integer32(i) => Ok(Self::Int(i)),
            boa_engine::JsVariant::BigInt(_) => Err(JsNativeError::typ()
                .with_message("cannot serialize BigInt")
                .into()),
            boa_engine::JsVariant::Object(js_object) => {
                if js_object.is_array() {
                    Ok(Self::Array(Vec::<JsValue>::try_from_js(
                        &js_object.into(),
                        context,
                    )?))
                } else {
                    Err(JsNativeError::typ()
                        .with_message("cannot serialize object")
                        .into())
                }
            }
            boa_engine::JsVariant::Symbol(js_symbol) => Ok(Self::String(
                js_symbol
                    .description()
                    .ok_or_else(|| {
                        JsNativeError::typ().with_message("cannot serialize blank symbol")
                    })?
                    .to_std_string()
                    .map_err(JsError::from_rust)?,
            )),
        }
    }
}

impl TryIntoJs for JsValue {
    fn try_into_js(&self, context: &mut Context) -> JsResult<boa_engine::JsValue> {
        match self {
            JsValue::Undefined => Ok(boa_engine::JsValue::undefined()),
            JsValue::Null => Ok(boa_engine::JsValue::null()),
            JsValue::Bool(b) => b.try_into_js(context),
            JsValue::Int(i) => i.try_into_js(context),
            JsValue::String(s) => s.try_into_js(context),
            JsValue::Array(values) => values.try_into_js(context),
            JsValue::Store(store_object) => store_object.try_into_js(context),
        }
    }
}

impl ToHash for JsValue {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        match self {
            JsValue::Undefined => hasher.update(b"RustValue::Undefined"),
            JsValue::Null => hasher.update(b"RustValue::Null"),
            JsValue::Bool(b) => {
                hasher.update(b"RustValue::Bool(");
                hasher.update([if *b { 255 } else { 0 }]);
                hasher.update(b")");
            }
            JsValue::Int(i) => {
                hasher.update(b"RustValue::Int(");
                hasher.update(i.to_le_bytes());
                hasher.update(b")");
            }
            JsValue::String(s) => {
                hasher.update(b"RustValue::String(");
                hasher.update(s.as_bytes());
                hasher.update(b")");
            }
            JsValue::Array(vs) => {
                hasher.update(b"RustValue::Array(");
                for v in vs.iter() {
                    v.run_hash(hasher);
                }
                hasher.update(b")");
            }
            JsValue::Store(store_object) => {
                hasher.update(b"RustValue::Store(");
                store_object.object.run_hash(hasher);
                hasher.update(b")");
            }
        }
    }
}

impl std::fmt::Display for JsValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsValue::Undefined => write!(f, "undefined"),
            JsValue::Null => write!(f, "null"),
            JsValue::Bool(b) => write!(f, "{}", if *b { "true" } else { "false" }),
            JsValue::Int(i) => write!(f, "{}", i),
            JsValue::String(s) => write!(f, "\"{}\"", s),
            JsValue::Array(vs) => {
                write!(f, "[")?;
                for (i, v) in vs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")?;
                Ok(())
            }
            JsValue::Store(store_object) => write!(f, "objects/{}", store_object.object),
        }
    }
}
