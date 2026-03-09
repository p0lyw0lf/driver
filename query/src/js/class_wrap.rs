#[macro_export]
macro_rules! class_wrap {
    (class $class:ident {
        length $length:literal,
        $(constructor $constructor:expr,)?
        $(methods {$(
            $method_name:ident: ($method_count:literal) $method_fn:expr,
        )*},)?
        $(statics {$(
            $static_name:ident: ($static_count:literal) $static_fn:expr,
        )*},)?
    }) => {
        impl boa_engine::class::Class for $class {
            const NAME: &'static str = stringify!($ident);
            const LENGTH: usize = $length;

            fn data_constructor(
                this: &boa_engine::JsValue,
                args: &[boa_engine::JsValue],
                context: &mut boa_engine::Context,
            ) -> JsResult<Self> {
                $(
                    return ($constructor)(this, args, context);
                )?
                Err(boa_engine::error::JsNativeError::typ().with_message(concat!(stringify!($class), " is not constructible")).into())
            }

            fn init(class: &mut boa_engine::class::ClassBuilder<'_>) -> JsResult<()> {
                fn wrap<T: boa_engine::value::TryIntoJs>(
                    f: fn(boa_gc::GcRef<'_, $class>, &[boa_engine::JsValue], &mut boa_engine::Context) -> boa_engine::JsResult<T>,
                ) -> fn(&boa_engine::JsValue, &[boa_engine::JsValue], &mut Context) -> boa_engine::JsResult<JsValue> {
                    |this, args, context| {
                        if let Some(this) = this.as_object()
                            && let Some(this) = this.downcast_ref::<$class>()
                        {
                            let out = f(this, args, context)?;
                            out.try_into_js(context)
                        } else {
                            Err(JsNativeError::typ()
                                .with_message("'this' is not a JsObject")
                                .into())
                        }
                    }
                }
                $($(
                    class.method(
                        boa_engine::js_string!(stringify!($method_name)),
                        $method_count,
                        boa_engine::NativeFunction::from_fn_ptr(wrap($method_fn)),
                    );
                )*)?
                $($(
                    class.static_method(
                        boa_engine::js_string!(stringify!($static_name)),
                        $static_count,
                        boa_engine::NativeFunction::from_fn_ptr(wrap($static_fn)),
                    );
                )*)?
            }
        }
    };
}
