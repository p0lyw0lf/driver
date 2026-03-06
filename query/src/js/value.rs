use rquickjs::{FromJs, IntoJs, Value};
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

impl<'js> FromJs<'js> for JsValue {
    fn from_js(ctx: &rquickjs::Ctx<'js>, value: rquickjs::Value<'js>) -> rquickjs::Result<Self> {
        match value.type_of() {
            rquickjs::Type::Uninitialized => Err(rquickjs::Error::Unknown),
            rquickjs::Type::Undefined => Ok(JsValue::Undefined),
            rquickjs::Type::Null => Ok(JsValue::Null),
            rquickjs::Type::Bool => Ok(JsValue::Bool(value.as_bool().unwrap())),
            rquickjs::Type::Int => Ok(JsValue::Int(value.as_int().unwrap())),
            rquickjs::Type::Float => Err(rquickjs::Error::FromJs {
                from: "Float",
                to: "RustValue",
                message: None,
            }),
            rquickjs::Type::String => Ok(JsValue::String(value.as_string().unwrap().to_string()?)),
            rquickjs::Type::Symbol => Ok(JsValue::String(
                value.as_symbol().unwrap().as_atom().to_string()?,
            )),
            rquickjs::Type::Array => Ok(JsValue::Array(Vec::from_js(ctx, value)?)),
            rquickjs::Type::Constructor => Err(rquickjs::Error::FromJs {
                from: "Constructor",
                to: "RustValue",
                message: None,
            }),
            rquickjs::Type::Function => Err(rquickjs::Error::FromJs {
                from: "Function",
                to: "RustValue",
                message: None,
            }),
            rquickjs::Type::Promise => Err(rquickjs::Error::FromJs {
                from: "Promise",
                to: "RustValue",
                message: None,
            }),
            rquickjs::Type::Exception => Err(rquickjs::Error::FromJs {
                from: "Exception",
                to: "RustValue",
                message: None,
            }),
            rquickjs::Type::Proxy => Err(rquickjs::Error::FromJs {
                from: "Proxy",
                to: "RustValue",
                message: None,
            }),
            rquickjs::Type::Object => {
                let object = value.as_object().unwrap();
                if let Some(cls) = object.as_class::<JsObject>() {
                    Ok(JsValue::Store(cls.borrow().clone()))
                } else {
                    Err(rquickjs::Error::FromJs {
                        from: "Object",
                        to: "RustValue",
                        message: None,
                    })
                }
            }
            rquickjs::Type::Module => Err(rquickjs::Error::FromJs {
                from: "Module",
                to: "RustValue",
                message: None,
            }),
            rquickjs::Type::BigInt => Err(rquickjs::Error::FromJs {
                from: "BigInt",
                to: "RustValue",
                message: None,
            }),
            rquickjs::Type::Unknown => Err(rquickjs::Error::Unknown),
        }
    }
}

impl<'js> IntoJs<'js> for JsValue {
    fn into_js(self, ctx: &rquickjs::Ctx<'js>) -> rquickjs::Result<rquickjs::Value<'js>> {
        match self {
            JsValue::Undefined => Ok(Value::new_uninitialized(ctx.clone())),
            JsValue::Null => Ok(Value::new_null(ctx.clone())),
            JsValue::Bool(b) => b.into_js(ctx),
            JsValue::Int(i) => i.into_js(ctx),
            JsValue::String(s) => s.into_js(ctx),
            JsValue::Array(values) => values.into_js(ctx),
            JsValue::Store(store_object) => store_object.into_js(ctx),
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
