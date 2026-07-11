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
/// See module documentation for an example/more information.
pub trait Producer<Key: ProducerBase>: ProducerBase {
    fn produce(&self, ctx: &Context<Key>) -> impl Future<Output = Self::Output>;
}

/// Helper macro to alleviate some of the boilerplate of writing `Producer` implementations.
///
/// Formatted mostly like a normal function declaration, except in the brackets we put all the
/// subqueries we use inside this function, in order to generate the appropriate trait bounds. See
/// module documentation for an example.
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
        )*)?
        {
            async fn produce(&$self, $ctx: &$crate::Context<Key>) -> $output { $($tt)* }
        }
    };

    ($name:ident ($self:ident, $ctx:ident) as ($query_key:ty) -> $output:ty { $($tt:tt)* }) => {
        impl $crate::ProducerBase for $name {
            type Output = $output;
        }
        impl $crate::Producer<$query_key> for $name {
            async fn produce(&$self, $ctx: &$crate::Context<$query_key>) -> $output { $($tt)* }
        }
    };
}

/// The main function that library authors should use in order to consume other incrementally-computed
/// values. It MUST be used instead of directly calling `.produce()`, both inside `produce()`
/// implementations and outside of them.
///
/// See module documentation for example usage.
pub async fn query<KSmall, KLarge>(ctx: &Context<KLarge>, key: KSmall) -> KSmall::Output
where
    KSmall: ProducerBase + Into<KLarge>,
    KLarge: Producer<KLarge>,
    KLarge::Output: Downcastable,
{
    query_with_hash(ctx, key).await.1
}

/// Like [`query`], but also returns a hashed representation of they key, for use in creating other
/// efficient datastructures.
pub async fn query_with_hash<KSmall, KLarge>(
    ctx: &Context<KLarge>,
    key: KSmall,
) -> (driver_db::Hashed<KLarge>, KSmall::Output)
where
    KSmall: ProducerBase + Into<KLarge>,
    KLarge: Producer<KLarge>,
    KLarge::Output: Downcastable,
{
    let (hash, output) = ctx
        .executor()
        .execute_pinned({
            let ctx = ctx.clone();
            let key = key.into();
            move || ctx.query_internal(key)
        })
        .await;

    (
        hash,
        output
            .downcast()
            .expect("query produced wrong type somehow"),
    )
}

pub trait Downcastable {
    /// Allows for downcasting this output into an output type of one of its subkeys.
    fn downcast<T: 'static>(self) -> Option<T>;
}

/// Turns a collection of producers into an enum that is compatible with `Queryable`.
///
/// See module documentation for an example.
#[macro_export]
macro_rules! query {
    ($name:ident { $(
        $key:ident
    ),* $(,)? } with $output:ident) => {
        $crate::key!(enum $name { $($key,)* });

        #[derive(PartialEq, Eq, Clone, Debug, serde::Serialize, serde::Deserialize)]
        pub enum $output { $(
            $key(<$key as $crate::ProducerBase>::Output),
        )* }

        impl $crate::BlobTrace for $output {
            fn trace(&self) -> impl Iterator<Item = &'_ $crate::Blob> {
                match self { $(
                    Self::$key(output) => Box::new($crate::BlobTrace::trace(output)) as Box<dyn Iterator<Item = &'_ $crate::Blob>>,
                )* }
            }
        }

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

        impl $crate::Downcastable for $output {
            /// Doesn't allocate, probably unsafe but who caressss
            fn downcast<T: 'static>(self) -> Option<T> {
                match self {
                $(
                    Self::$key(output) if std::any::Any::type_id(&output) == std::any::TypeId::of::<T>() => {
                        // SAFETY: The types match exactly, transmute in-place.
                        unsafe {
                            // We're about to lose ownership after casting to a safe pointer &
                            // dropping, but we want the value read afterwards to presist. So we
                            // need to stop this drop from happening.
                            let output = std::mem::ManuallyDrop::new(output);
                            // This cast is _probably_ unsafe UB, but that's why we have this block
                            // I guess (:
                            let output = std::ptr::read((&*output as *const <$key as $crate::ProducerBase>::Output).cast::<T>());
                            Some(output)
                        }
                    }
                )*
                    _ => None,
                }
            }
        }
    }
}
