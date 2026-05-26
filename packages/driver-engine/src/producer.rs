use crate::Context;

/// A trait used to provide "virtual" access to a context, so as to resolve some pretty gnarly
/// recursive trait definitions

/// The main trait that library authors should implement to support incremental compilation. The
/// `Key` type parameter is so that a single `Self` can generate multiple implementations for
/// different "container keys" that it works in tandem with.
///
///
/// The trait bounds on the `produce()` function are needed because otherwise we run into an
/// infinite loop trying to resolve `Key: Producer<Key>`.
///
/// See `query` for an example/more information.
pub trait Producer: driver_util::Key {
    type Output: driver_util::Output + Sized + 'static;
    fn produce(&self, ctx: &Context<Key, Output>) -> impl Future<Output = Self::Output>;
}

/// Helper macro to alleviate some of the boilerplate of writing `Producer` implementations.
///
/// Formatted mostly like a normal function declaration, except in the brackets we put all the
/// subqueries we use inside this function, in order to generate the appropriate trait bounds. See
/// `query()` for an example.
#[macro_export]
macro_rules! producer {
    ($name:ident ($self:ident, $ctx:ident) $([ $( $subkey:ident ),* ])? -> $output:ident $tt:tt) => {
        impl<Key: driver_util::Key, Output: driver_util::Output> $crate::Producer<Key, Output> for $name
        where
        $($(
            $subkey: Into<Key>,
            $subkey: $crate::Producer<Key, Output>,
            Output: TryInto<<$subkey as $crate::Producer<Key, Output>>::Output>,
        )*)?
        {
            type Output = $output;
            async fn produce(&$self, $ctx: &$crate::Context<Key, Output>) -> $output $tt
        }
    }
}

/// The main function that library authors should use in order to consume other incrementally-computed
/// values. It MUST be used instead of directly calling `.produce()`.
pub async fn query<K, Key: driver_util::Key, Output: driver_util::Output>(
    ctx: &Context<Key, Output>,
    key: K,
) -> K::Output
where
    K: Producer<Key, Output>,
{
    /*
        let value = ctx
            .executor()
            .execute_pinned({
                let ctx = ctx.clone();
                let key = self.into();
                move || ctx.query_internal(key)
            })
            .await;
        value.try_into().expect("query produced wrong type somehow")
    */
    todo!()
}

#[cfg(test)]
mod test {
    #[test]
    fn fib() {
        use crate::{Context, query};

        crate::key!(
            #[input=|_| false]
            struct Fib(u32);
        );
        crate::producer!(Fib(self, ctx) [Fib] -> u32 {
            /*
            let n = self.0;
            if n == 0 || n == 1 {
                return 1;
            }

            let n_1 = query(ctx, Fib(n-1)).await;
            let n_2 = query(ctx, Fib(n-2)).await;

            n_1 + n_2
            */
            100
        });

        let ctx = Context::<Fib, _>::create_empty_root_for_testing_only();
        let output = futures_lite::future::block_on(query(&ctx, Fib(10)));
        assert_eq!(output, 100);

        impl std::fmt::Display for Fib {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "Fib({})", self.0)
            }
        }
    }
}

/// Turns a collection of producers into an enum that is compatible with `Queryable`.
///
/// Example:
///
/// ```rust
///
/// ```
#[macro_export]
macro_rules! query_key {
    ($name:ident { $(
        $key:ident ,
    )* } with $output:ident) => {
        $crate::key!(enum $name { $($key,)* });

        pub enum $output { $(
            $key(<$key as $crate::Producer>::Output),
        )* }

        impl<Key> $crate::Producer<Key> for $name
        where
            Key: $crate::Producer<Key>,
            $name: Into<Key>,
        {
            type Output = $output;
            async fn produce(&self, ctx: &$crate::Context<Key>) -> Self::Output {
                // Dispatch to appropriate producer based on enum tag, wrapping output in the same.
                match self { $(
                    // Doesn't have to use `query`, because it's kept track of the same way.
                    Self::$key(key) => $output::$key(key.produce(ctx).await),
                )* }
            }
        }

        // Allow for "downcasting" from the output back to the original outputs.
        $(
        impl TryFrom<$output> for <$key as $crate::Producer>::Output {
            type Error = ();
            fn try_from(output: $output) -> Result<Self, Self::Error> {
                if let $output::$key(output) = output {
                    Ok(output)
                } else {
                    Err(())
                }
            }
        }
        )*
    }
}
