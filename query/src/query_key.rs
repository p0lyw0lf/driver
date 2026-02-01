use dashmap::DashMap;

use crate::HashDirectory;
use crate::HashFile;
use crate::MemoizeKotoFunction;
use crate::RunKotoFunction;
use crate::files::ListDirectory;
use crate::files::ReadFile;
use crate::to_hash::ToHash;

macro_rules! query_key {
    ($key:ident ($cache:ident) { $(
        $name:ident : $type:ident ,
    )* }) => {
        #[derive(Hash, PartialEq, Eq, Clone, Debug)]
        pub enum $key {
            $($type($type),)*
        }

        $(
        impl From<$type> for $key {
            fn from(v: $type) -> Self {
                Self::$type(v)
            }
        }
        )*


        impl $crate::Producer for $key {
            type Output = $crate::AnyOutput;
            fn produce(&self, ctx: &$crate::QueryContext) -> anyhow::Result<Self::Output> {
                Ok(match self { $(
                    Self::$type(v) => $crate::AnyOutput::new(v.produce(ctx)?),
                )* })
            }
        }

        #[derive(Clone, Debug, Default)]
        pub struct $cache { $(
            pub $name: DashMap<$type, <$type as $crate::Producer>::Output>,
        )* }

        impl $cache {
            /// REQUIRES: value was produced by key.
            /// RETURNS: whether cache was busted, that is, whether the cache changed based on the
            /// new value.
            pub fn insert(&self, key: QueryKey, value: $crate::AnyOutput) -> bool {
                match key { $(
                    $key::$type(key) => {
                        let value: <$type as $crate::Producer>::Output = *value.downcast().expect("must be produced by key");
                        let hash = value.to_hash();
                        let old = self.$name.insert(key, value);
                        old.is_none_or(|old| old.to_hash() == hash)
                    }
                )* }
            }

            pub fn get(&self, key: &QueryKey) -> Option<$crate::AnyOutput> {
                match key { $(
                    $key::$type(key) => self.$name.get(key).map(|v| $crate::AnyOutput::new(v.clone())),
                )* }
            }
        }
    }
}

query_key!(QueryKey (QueryCache) {
    // long-term things
    read_file: ReadFile,
    list_directory: ListDirectory,
    memoize_koto_function: MemoizeKotoFunction,
    run_koto_function: RunKotoFunction,
    // short-term things to help with testing
    hash_file: HashFile,
    hash_directory: HashDirectory,
});

impl QueryKey {
    // whether a new revision should cause this key to be immediately re-computed or not
    pub fn is_input(&self) -> bool {
        match self {
            QueryKey::ReadFile(_) | QueryKey::ListDirectory(_) => true,
            QueryKey::HashFile(_)
            | QueryKey::HashDirectory(_)
            | QueryKey::MemoizeKotoFunction(_)
            | QueryKey::RunKotoFunction(_) => false,
        }
    }
}
