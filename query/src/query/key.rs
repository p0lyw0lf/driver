use std::fmt::Display;

use dashmap::DashMap;
use serde::Deserialize;
use serde::Serialize;

use crate::js::MarkdownToHtml;
use crate::js::MinifyHtml;
use crate::js::RunFile;
use crate::query::files::ListDirectory;
use crate::query::files::ReadFile;
use crate::to_hash::ToHash;

#[macro_export]
macro_rules! query_key {
    ($name:ident $tt:tt) => {
        #[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
        pub struct $name $tt
    };
    ($name:ident $tt:tt ;) => {
        #[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
        pub struct $name $tt ;
    };
}

macro_rules! query_keys {
    ($key:ident ($cache:ident) { $(
        $name:ident : $type:ident ,
    )* }) => {
        #[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
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
            type Output = $crate::query::context::AnyOutput;
            fn produce(&self, ctx: &$crate::query::context::QueryContext) -> Self::Output {
                match self { $(
                    Self::$type(v) => $crate::query::context::AnyOutput::new(v.produce(ctx)),
                )* }
            }
        }

        // We put the cache in here too so that it can be Serialize/Deserialize without doing
        // anything crazier with AnyOutput
        #[derive(Clone, Debug, Default, Serialize, Deserialize)]
        pub struct $cache { $(
            pub $name: DashMap<$type, <$type as $crate::Producer>::Output>,
        )* }

        impl $cache {
            /// REQUIRES: value was produced by key.
            /// RETURNS: whether cache was busted, that is, whether the cache changed based on the
            /// new value.
            pub fn insert(&self, key: QueryKey, value: $crate::query::context::AnyOutput) -> bool {
                match key { $(
                    $key::$type(key) => {
                        let value: <$type as $crate::query::context::Producer>::Output = *value.downcast().expect("must be produced by key");
                        let hash = value.to_hash();
                        let old = self.$name.insert(key, value);
                        old.is_none_or(|old| old.to_hash() == hash)
                    }
                )* }
            }

            pub fn get(&self, key: &QueryKey) -> Option<$crate::query::context::AnyOutput> {
                match key { $(
                    $key::$type(key) => self.$name.get(key).map(|v| $crate::query::context::AnyOutput::new(v.clone())),
                )* }
            }

            pub fn iter_keys(&self) -> impl std::iter::Iterator<Item = QueryKey> {
                std::iter::empty()
                $(.chain(
                    self.$name.iter()
                    .map(|entry| {
                        QueryKey::$type(entry.key().clone())
                    })
                ))*
            }
        }
    }
}

query_keys!(QueryKey (QueryCache) {
    read_file: ReadFile,
    list_directory: ListDirectory,
    run_file: RunFile,
    minify_html: MinifyHtml,
    markdown_to_html: MarkdownToHtml,
});

impl QueryKey {
    // whether a new revision should cause this key to be immediately re-computed or not
    pub fn is_input(&self) -> bool {
        match self {
            QueryKey::ReadFile(_) => true,
            QueryKey::ListDirectory(_) => true,
            QueryKey::RunFile(_) => false,
            QueryKey::MinifyHtml(_) => false,
            QueryKey::MarkdownToHtml(_) => false,
        }
    }
}

impl Display for QueryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryKey::ReadFile(read_file) => write!(f, "read_file({:?})", read_file.0),
            QueryKey::ListDirectory(list_directory) => {
                write!(f, "list_directory({:?})", list_directory.0)
            }
            QueryKey::RunFile(run_file) => {
                write!(
                    f,
                    "{}({})",
                    run_file.file.display(),
                    run_file
                        .args
                        .as_ref()
                        .map(|arg| arg.to_string())
                        .unwrap_or_default(),
                )
            }
            QueryKey::MinifyHtml(minify_html) => write!(f, "minify_html({})", minify_html.0),
            QueryKey::MarkdownToHtml(markdown_to_html) => {
                write!(f, "markdown_to_html({})", markdown_to_html.0)
            }
        }
    }
}
