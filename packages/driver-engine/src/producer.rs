use crate::Context;

/// A trait that registers a key with the potential to produce exactly **one** output type. Not
/// part of `Producer` in order to resolve some nasty trait recursion things.
pub trait ProducerBase: driver_util::Key {
    type Output: driver_util::Output;
}

/// The main trait that library authors should implement to support incremental compilation. The
/// `Key` type parameter is so that a single `Self` can generate multiple implementations for
/// different "container keys" that it works in tandem with.
///
/// See `query` for an example/more information.
pub trait Producer<Key: ProducerBase>: ProducerBase {
    fn produce(&self, ctx: &Context<Key>) -> impl Future<Output = Self::Output>;
}

/// Helper macro to alleviate some of the boilerplate of writing `Producer` implementations.
///
/// Formatted mostly like a normal function declaration, except in the brackets we put all the
/// subqueries we use inside this function, in order to generate the appropriate trait bounds. See
/// `query()` for an example.
#[macro_export]
macro_rules! producer {
    ($name:ident ($self:ident, $ctx:ident) $(where [ $( $subkey:ident ),* ])? -> $output:ty { $($tt:tt)* }) => {
        impl $crate::ProducerBase for $name {
            type Output = $output;
        }
        impl<Key> $crate::Producer<Key> for $name
        where
            Key: $crate::Producer<Key>,
        $($(
            $subkey: Into<Key>,
            Key::Output: TryInto<<$subkey as $crate::ProducerBase>::Output>,
        )*)?
        {
            async fn produce(&$self, $ctx: &$crate::Context<Key>) -> $output { $($tt)* }
        }
    }
}

/// The main function that library authors should use in order to consume other incrementally-computed
/// values. It MUST be used instead of directly calling `.produce()`, both inside `produce()`
/// implementations and outside of them.
pub async fn query<KSmall, KLarge>(ctx: &Context<KLarge>, key: KSmall) -> KSmall::Output
where
    KSmall: ProducerBase + Into<KLarge>,
    KLarge: Producer<KLarge>,
    KLarge::Output: TryInto<KSmall::Output>,
{
    let value = ctx
        .executor()
        .execute_pinned({
            let ctx = ctx.clone();
            let key = key.into();
            move || ctx.query_internal(key)
        })
        .await;
    // IMPORTANT: We must do `.ok()` first to get rid of the error, because providing `Debug`
    // bounds leads to some very strange weirdness: https://github.com/rust-lang/rust/issues/156998
    value
        .try_into()
        .ok()
        .expect("query produced wrong type somehow")
}

/// Turns a collection of producers into an enum that is compatible with `Queryable`.
///
/// See module documentation for an example.
#[macro_export]
macro_rules! query_key {
    ($name:ident { $(
        $key:ident
    ),* } with $output:ident) => {
        $crate::key!(enum $name { $($key,)* });

        #[derive(PartialEq, Eq, Clone, Debug, serde::Serialize, serde::Deserialize)]
        pub enum $output { $(
            $key(<$key as $crate::ProducerBase>::Output),
        )* }

        impl $crate::ProducerBase for $name {
            type Output = $output;
        }

        impl $crate::Producer<$name> for $name {
            /// Dispatch to appropriate producer based on enum tag, wrapping output in the same.
            async fn produce(&self, ctx: &$crate::Context<Self>) -> $output {
                match self { $(
                    // Doesn't have to use `query`, because it's kept track of the same way.
                    Self::$key(key) => $output::$key(key.produce(ctx).await),
                )* }
            }
        }

        $(
        /// Allows for "downcasting" from the collected output back to the original outputs.
        impl TryFrom<$output> for <$key as $crate::ProducerBase>::Output {
            type Error = ();
            fn try_from(output: $output) -> Result<Self, Self::Error> {
                #[allow(irrefutable_let_patterns)]
                let $output::$key(output) = output else {
                    return Err(());
                };
                Ok(output)
            }
        }
        )*
    }
}
