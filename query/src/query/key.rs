use std::fmt::Display;

use serde::Deserialize;
use serde::Serialize;

use crate::js::GetUrl;
use crate::js::MarkdownToHtml;
use crate::js::MinifyHtml;
use crate::js::RunFile;
use crate::query::files::ListDirectory;
use crate::query::files::ReadFile;

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

macro_rules! query_key {
    ($key:ident { $(
        $type:ident ,
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
            async fn produce(&self, ctx: &$crate::query::context::QueryContext) -> Self::Output {
                match self { $(
                    Self::$type(v) => $crate::query::context::AnyOutput::new(v.produce(ctx).await),
                )* }
            }
        }
    }
}

query_key!(QueryKey {
    GetUrl,
    ListDirectory,
    MarkdownToHtml,
    MinifyHtml,
    ReadFile,
    RunFile,
});

impl QueryKey {
    // whether a new revision should cause this key to be immediately re-computed or not
    pub fn is_input(&self) -> bool {
        match self {
            QueryKey::GetUrl(_) => true,
            QueryKey::ListDirectory(_) => true,
            QueryKey::MarkdownToHtml(_) => false,
            QueryKey::MinifyHtml(_) => false,
            QueryKey::ReadFile(_) => true,
            QueryKey::RunFile(_) => false,
        }
    }
}

impl Display for QueryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryKey::GetUrl(url) => write!(f, "get_url({})", url.0),
            QueryKey::ListDirectory(list_directory) => {
                write!(f, "list_directory({:?})", list_directory.0)
            }

            QueryKey::MarkdownToHtml(markdown_to_html) => {
                write!(f, "markdown_to_html({})", markdown_to_html.0)
            }
            QueryKey::MinifyHtml(minify_html) => write!(f, "minify_html({})", minify_html.0),
            QueryKey::ReadFile(read_file) => write!(f, "read_file({:?})", read_file.0),
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
        }
    }
}
