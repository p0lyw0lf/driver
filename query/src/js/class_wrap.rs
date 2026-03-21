#[macro_export]
macro_rules! class_fn_wrap {
    ($class:ident, $f:expr) => {
        |this: &boa_engine::JsValue,
         args: &[boa_engine::JsValue],
         mut context: &mut boa_engine::Context| {
            if let Some(this) = this.as_object()
                && let Some(this) = this.downcast_ref::<$class>()
            {
                let out = ($f)(this, args, &mut context)?;
                boa_engine::value::TryIntoJs::try_into_js(&out, context)
            } else {
                Err(boa_engine::JsNativeError::typ()
                    .with_message("'this' is not a JsObject")
                    .into())
            }
        }
    };
}

#[macro_export]
macro_rules! class_wrap {
    (class $class:ident {
        length $length:literal,
        $(constructor $constructor:expr,)?
        $(methods {$(
            $method_name:ident: ($method_count:literal) $method_fn:expr,
        )*},)?
    }) => {
        impl boa_engine::value::TryFromJs for $class {
            fn try_from_js(value: &boa_engine::value::JsValue, _js_ctx: &mut boa_engine::context::Context) -> boa_engine::JsResult<Self> {
                let object = value.as_object().ok_or_else(||
                    boa_engine::error::JsNativeError::typ()
                        .with_message(concat!(stringify!($class), " must be object"))
                )?;
                let this = object.downcast_ref::<$class>().ok_or_else(||
                    boa_engine::error::JsNativeError::typ()
                        .with_message(concat!("object is not ", stringify!($class)))
                )?;
                Ok(this.clone())
            }
        }

        impl boa_engine::class::Class for $class {
            const NAME: &'static str = stringify!($class);
            const LENGTH: usize = $length;

            fn data_constructor(
                _this: &boa_engine::JsValue,
                _args: &[boa_engine::JsValue],
                _js_ctx: &mut boa_engine::Context,
            ) -> JsResult<Self> {
                $(
                    return ($constructor)(_this, _args, _context);
                )?
                Err(boa_engine::error::JsNativeError::typ().with_message(concat!(stringify!($class), " is not constructible")).into())
            }

            fn init(class: &mut boa_engine::class::ClassBuilder<'_>) -> JsResult<()> {
                $($(
                    class.method(
                        boa_engine::js_string!(stringify!($method_name)),
                        $method_count,
                        boa_engine::NativeFunction::from_fn_ptr($crate::class_fn_wrap!($class, $method_fn)),
                    );
                )*)?
                Ok(())
            }
        }
    };
}
