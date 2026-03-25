// From https://danielkeep.github.io/tlborm/book/blk-counting.html#slice-length
macro_rules! replace_expr {
    ($i:ident, $sub:expr) => {
        $sub
    };
}
pub(crate) use replace_expr;

macro_rules! count_args {
        ($($arg:ident),*) => { <[()]>::len(&[$($crate::js::macros::replace_expr!($arg, ())),*]) };
    }
pub(crate) use count_args;

macro_rules! fn_body {
        ($fn:ident ($(
            $arg:ident : $ty:ty
        ),* $(,
            [$ctx:ident]
        )?) -> $ret:ty) => {
            |_this, _args, js_ctx| {
                let _i = 0;
                $(
                    let $arg: $ty = boa_engine::value::TryFromJs::try_from_js(
                        boa_engine::JsArgs::get_or_undefined(_args, _i),
                        js_ctx,
                    )?;
                    let _i = _i + 1;
                )*
                let out = {
                    $fn($($arg),* $(, {
                        let $ctx = js_ctx;
                        $ctx
                    })?)
                }?;
                boa_engine::value::TryIntoJs::try_into_js(&out, js_ctx)
            }
        }
    }
pub(crate) use fn_body;

macro_rules! async_fn_body {
        ($fn:ident ($(
            $arg:ident : $ty:ty
        ),* $(,
            [$ctx:ident]
        )?) -> $ret:ty) => {
            async |_this, _args, js_ctx| {
                let _i = 0;
                $(
                    let $arg: $ty = boa_engine::value::TryFromJs::try_from_js(
                        boa_engine::JsArgs::get_or_undefined(_args, _i),
                        &mut *js_ctx.borrow_mut(),
                    )?;
                    let _i = _i + 1;
                )*
                let out = {
                    $fn($($arg),* $(, {
                        let $ctx = js_ctx;
                        $ctx
                    })?)
                }.await?;
                boa_engine::value::TryIntoJs::try_into_js(&out, &mut *js_ctx.borrow_mut())
            }
        };
    }
pub(crate) use async_fn_body;

macro_rules! fn_obj {
        ($js_ctx:ident : $fn:ident ($(
            $arg:ident : $ty:ty
        ),* $(,
            [$ctx:ident : &mut Context]
        )? $(,)?) -> $ret:ty) => {
            boa_engine::object::FunctionObjectBuilder::new(
                $js_ctx.realm(),
                boa_engine::native_function::NativeFunction::from_fn_ptr(
                    $crate::js::macros::fn_body!($fn($($arg: $ty),* $(, [$ctx])?) -> $ret)
                ),
            )
            .length($crate::js::macros::count_args!($($arg),*))
            .name(stringify!($fn))
            .build()
        };
    }
pub(crate) use fn_obj;

macro_rules! async_fn_obj {
        ($js_ctx:ident : $fn:ident ($(
            $arg:ident : $ty:ty
        ),* $(,
            [$ctx:ident : &mut Context]
        )? $(,)?) -> $ret:ty) => {
            boa_engine::object::FunctionObjectBuilder::new(
                $js_ctx.realm(),
                boa_engine::native_function::NativeFunction::from_async_fn(
                    $crate::js::macros::async_fn_body!($fn($($arg: $ty),* $(, [$ctx])?) -> $ret)
                ),
            )
            .length($crate::js::macros::count_args!($($arg),*))
            .name(stringify!($fn))
            .build()
        };
    }
pub(crate) use async_fn_obj;

macro_rules! module {
        (use $js_ctx:ident;
        $(
            $(fn $fn:ident ($($tts:tt)*) -> JsResult<$ret:ty>)?
            $(async fn $async_fn:ident ($($async_tts:tt)*) -> JsResult<$async_ret:ty>)?
            ;
        )*) => {
            {
            $(
                $(let $fn = $crate::js::macros::fn_obj!($js_ctx : $fn($($tts)*) -> $ret);)?
                $(let $async_fn = $crate::js::macros::async_fn_obj!($js_ctx : $async_fn($($async_tts)*) -> $async_ret);)?
            )*
            boa_engine::module::Module::synthetic(
                &[$(
                    $(boa_engine::js_string!(stringify!($fn)),)?
                    $(boa_engine::js_string!(stringify!($async_fn)),)?
                )*],
                boa_engine::module::SyntheticModuleInitializer::from_copy_closure_with_captures(
                    |module, fns, _| {
                        let ($(
                                $($fn)?
                                $($async_fn)?
                            ),*) = fns;
                        $(
                            $(module.set_export(
                                &boa_engine::js_string!(stringify!($fn)),
                                $fn.clone().into(),
                            )?;)?
                            $(module.set_export(
                                &boa_engine::js_string!(stringify!($async_fn)),
                                $async_fn.clone().into(),
                            )?;)?
                        )*
                        Ok(())
                    },
                    ($(
                        $($fn)?
                        $($async_fn)?
                    ),*),
                ),
                None,
                None,
                $js_ctx,
            )
            }
        }
    }
pub(crate) use module;

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
pub(crate) use class_fn_wrap;

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
                        boa_engine::NativeFunction::from_fn_ptr($crate::js::macros::class_fn_wrap!($class, $method_fn)),
                    );
                )*)?
                Ok(())
            }
        }
    };
}
pub(crate) use class_wrap;
