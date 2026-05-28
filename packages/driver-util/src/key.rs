use std::fmt::{Debug, Display};
use std::hash::Hash;

/// Marker trait that MUST be implemented for all keys present in the system.
pub trait Key:
    'static
    + Send
    + Sync
    + Hash
    + Eq
    + Ord
    + Clone
    + Debug
    + Display
    + serde::Serialize
    + for<'de> serde::Deserialize<'de>
{
    /// Returns whether a given key is an "input" or not. Being an input key has a special
    /// connotation: input keys are assumed to not have any dependencies, and are instead _only_
    /// checked for if they match the current revision. Normally, a key with no dependencies (like minifying HTML, or resizing an image) will
    /// always be up-to-date (because everything they can possibly do is provided by the key
    /// itself), whereas input keys interact with some external system (network, filesystem).
    fn is_input(&self) -> bool;
}

/// Helper that allows you to define query keys that derive all the appropriate marker trait `Key`.
/// Said trait can also be implemented manually, but this macro allows it to be implemented without
/// all the extra boilerplate that would normally be required.
///
/// Example:
///
/// ```rust
/// use driver_util::{key, Key as _};
///
/// key!(#[input=|_| true] struct Foo;);
/// key!(#[input=|_| false] struct Bar(i32););
/// key!(#[input=|this: &Baz| this.x == this.y] struct Baz {
///     x: i32,
///     y: i32,
/// });
/// key!(enum Qux {
///     Foo,
///     Bar,
///     Baz,
/// });
///
/// assert_eq!(Qux::Foo(Foo).is_input(), true);
/// assert_eq!(Qux::Bar(Bar(1337)).is_input(), false);
/// assert_eq!(Qux::Baz(Baz { x: 6, y: 9 }).is_input(), false);
/// assert_eq!(Qux::Baz(Baz { x: 7, y: 7 }).is_input(), true);
///
/// impl std::fmt::Display for Foo {
///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
///         f.write_str("Foo")
///     }
/// }
/// impl std::fmt::Display for Bar {
///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
///         write!(f, "Bar({})", self.0)
///     }
/// }
/// impl std::fmt::Display for Baz {
///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
///         write!(f, "Baz {{ x: {}, y: {} }}", self.x, self.y)
///     }
/// }
/// // `impl Display for Qux` is derived automatically.
/// ```
#[macro_export]
macro_rules! key {
    (#[ input = $input:expr ] struct $name:ident $tt:tt) => {
        #[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug, serde::Serialize, serde::Deserialize)]
        pub struct $name $tt

        impl $crate::Key for $name {
            fn is_input(&self) -> bool {
                ($input)(self)
            }
        }
    };
    // I wish Rust had a way of saying "this token is optional, but we want to condition on it
    // later"...
    (#[ input = $input:expr ] struct $name:ident $($tt:tt)? ;) => {
        #[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug, serde::Serialize, serde::Deserialize)]
        pub struct $name $($tt)?;

        impl $crate::Key for $name {
            fn is_input(&self) -> bool {
                ($input)(self)
            }
        }
    };

    (enum $name:ident { $(
        $key:ident ,
    )* }) => {
        #[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug, serde::Serialize, serde::Deserialize)]
        pub enum $name{
            $($key($key),)*
        }


        $(
        impl From<$key> for $name {
            fn from(v: $key) -> Self {
                Self::$key(v)
            }
        }
        )*

        impl $crate::Key for $name {
            fn is_input(&self) -> bool {
                match self { $(
                    Self::$key(x) => return x.is_input(),
                )* };
                // Just in case the enum is empty
                #[allow(unreachable_code)]
                false
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self { $(
                    Self::$key(x) => return std::fmt::Display::fmt(x, f),
                )* };
                // Just in case the enum is empty
                #[allow(unreachable_code)]
                Ok(())
            }
        }
    };
}
