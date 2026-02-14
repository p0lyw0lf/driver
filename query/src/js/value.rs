use rquickjs::{FromJs, IntoJs, Value as JsValue};
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::js::StoreObject;
use crate::to_hash::ToHash;

/// All the simple javascript values that can be serialized/deserialized losslessly
#[derive(Hash, PartialEq, Eq, Debug, Clone, Serialize, Deserialize)]
pub enum RustValue {
    Undefined,
    Null,
    Bool(bool),
    Int(i32),
    String(String),
    Array(Vec<RustValue>),
    Store(StoreObject),
}

impl<'js> FromJs<'js> for RustValue {
    fn from_js(ctx: &rquickjs::Ctx<'js>, value: rquickjs::Value<'js>) -> rquickjs::Result<Self> {
        match value.type_of() {
            rquickjs::Type::Uninitialized => Err(rquickjs::Error::Unknown),
            rquickjs::Type::Undefined => Ok(RustValue::Undefined),
            rquickjs::Type::Null => Ok(RustValue::Null),
            rquickjs::Type::Bool => Ok(RustValue::Bool(value.as_bool().unwrap())),
            rquickjs::Type::Int => Ok(RustValue::Int(value.as_int().unwrap())),
            rquickjs::Type::Float => Err(rquickjs::Error::FromJs {
                from: "Float",
                to: "RustValue",
                message: None,
            }),
            rquickjs::Type::String => {
                Ok(RustValue::String(value.as_string().unwrap().to_string()?))
            }
            rquickjs::Type::Symbol => Ok(RustValue::String(
                value.as_symbol().unwrap().as_atom().to_string()?,
            )),
            rquickjs::Type::Array => Ok(RustValue::Array(Vec::from_js(ctx, value)?)),
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
                if let Some(cls) = object.as_class::<StoreObject>() {
                    Ok(RustValue::Store(cls.borrow().clone()))
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

impl<'js> IntoJs<'js> for RustValue {
    fn into_js(self, ctx: &rquickjs::Ctx<'js>) -> rquickjs::Result<rquickjs::Value<'js>> {
        match self {
            RustValue::Undefined => Ok(JsValue::new_uninitialized(ctx.clone())),
            RustValue::Null => Ok(JsValue::new_null(ctx.clone())),
            RustValue::Bool(b) => b.into_js(ctx),
            RustValue::Int(i) => i.into_js(ctx),
            RustValue::String(s) => s.into_js(ctx),
            RustValue::Array(values) => values.into_js(ctx),
            RustValue::Store(store_object) => store_object.into_js(ctx),
        }
    }
}

impl ToHash for RustValue {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        match self {
            RustValue::Undefined => hasher.update(b"RustValue::Undefined"),
            RustValue::Null => hasher.update(b"RustValue::Null"),
            RustValue::Bool(b) => {
                hasher.update(b"RustValue::Bool(");
                hasher.update([if *b { 255 } else { 0 }]);
                hasher.update(b")");
            }
            RustValue::Int(i) => {
                hasher.update(b"RustValue::Int(");
                hasher.update(i.to_le_bytes());
                hasher.update(b")");
            }
            RustValue::String(s) => {
                hasher.update(b"RustValue::String(");
                hasher.update(s.as_bytes());
                hasher.update(b")");
            }
            RustValue::Array(vs) => {
                hasher.update(b"RustValue::Array(");
                for v in vs.iter() {
                    v.run_hash(hasher);
                }
                hasher.update(b")");
            }
            RustValue::Store(store_object) => {
                hasher.update(b"RustValue::Store(");
                store_object.object.run_hash(hasher);
                hasher.update(b")");
            }
        }
    }
}

impl std::fmt::Display for RustValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RustValue::Undefined => write!(f, "undefined"),
            RustValue::Null => write!(f, "null"),
            RustValue::Bool(b) => write!(f, "{}", if *b { "true" } else { "false" }),
            RustValue::Int(i) => write!(f, "{}", i),
            RustValue::String(s) => write!(f, "\"{}\"", s),
            RustValue::Array(vs) => {
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
            RustValue::Store(store_object) => write!(f, "objects/{}", store_object.object),
        }
    }
}
