use dashmap::DashMap;

use crate::files::ListDirectory;
use crate::files::ReadFile;
use crate::to_hash::ToHash;
use crate::HashDirectory;
use crate::HashFile;

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
            /// RETURNS: whether value was already present in the cache.
            pub fn insert(&self, key: QueryKey, value: $crate::AnyOutput) -> bool {
                match key { $(
                    $key::$type(key) => {
                        let value: <$type as $crate::Producer>::Output = *value.downcast().expect("must be produced by key");
                        let hash = value.to_hash();
                        let old = self.$name.insert(key, value);
                        old.is_some_and(|old| old.to_hash() == hash)
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
    // short-term things to help with testing
    hash_file: HashFile,
    hash_directory: HashDirectory,
});
