use std::ops::Deref;

impl TryFromJs for JsValue {
    fn try_from_js(value: &boa_engine::JsValue, js_ctx: &mut Context) -> JsResult<Self> {
        match value.variant() {
            boa_engine::JsVariant::Null => Ok(Self::Null),
            boa_engine::JsVariant::Undefined => Ok(Self::Undefined),
            boa_engine::JsVariant::Boolean(b) => Ok(Self::Bool(b)),
            boa_engine::JsVariant::String(js_string) => Ok(Self::String(
                js_string.to_std_string().map_err(JsError::from_rust)?,
            )),
            boa_engine::JsVariant::Float64(f) => {
                let i = f as i32;
                if (i as f64) == f {
                    Ok(Self::Int(i))
                } else {
                    Err(JsNativeError::typ()
                        .with_message(format!("cannot serialize float {f}"))
                        .into())
                }
            }
            boa_engine::JsVariant::Integer32(i) => Ok(Self::Int(i)),
            boa_engine::JsVariant::BigInt(_) => Err(JsNativeError::typ()
                .with_message("cannot serialize BigInt")
                .into()),
            boa_engine::JsVariant::Object(js_object) => {
                if js_object.is_array() {
                    Ok(Self::Array(Vec::<JsValue>::try_from_js(
                        &js_object.into(),
                        js_ctx,
                    )?))
                } else if let Some(object) = js_object.downcast_ref::<JsObject>() {
                    Ok(Self::Store(object.clone()))
                } else if let Some(image) = js_object.downcast_ref::<JsImage>() {
                    Ok(Self::Image(image.clone()))
                } else if js_object.is_ordinary() {
                    let mut out = BTreeMap::new();
                    for key in js_object.own_property_keys(js_ctx)? {
                        let string_key = key.to_string();
                        let value = js_object.get(key, js_ctx)?;
                        let _ = out.insert(string_key, JsValue::try_from_js(&value, js_ctx)?);
                    }
                    Ok(Self::Object(out))
                } else {
                    Err(JsNativeError::typ()
                        .with_message("cannot serialize unordinary object")
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
    fn try_into_js(&self, js_ctx: &mut Context) -> JsResult<boa_engine::JsValue> {
        match self {
            JsValue::Undefined => Ok(boa_engine::JsValue::undefined()),
            JsValue::Null => Ok(boa_engine::JsValue::null()),
            JsValue::Bool(b) => b.try_into_js(js_ctx),
            JsValue::Int(i) => i.try_into_js(js_ctx),
            JsValue::String(s) => s.try_into_js(js_ctx),
            JsValue::Array(values) => values.try_into_js(js_ctx),
            JsValue::Store(store_object) => store_object.try_into_js(js_ctx),
            JsValue::Image(image) => image.try_into_js(js_ctx),
            JsValue::Object(btree_map) => {
                let object = boa_engine::JsObject::with_object_proto(js_ctx.intrinsics());
                for (key, value) in btree_map.iter() {
                    let value = value.try_into_js(js_ctx)?;
                    object.set(boa_engine::JsString::from(key.deref()), value, true, js_ctx)?;
                }
                Ok(object.into())
            }
        }
    }
}
