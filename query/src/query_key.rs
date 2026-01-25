use crate::files::ListDirectory;
use crate::files::ReadFile;

macro_rules! query_key {
    ($key:ident { $(
        $t:ident ,
    )* }) => {
        #[derive(Hash, PartialEq, Eq, Clone, Debug)]
        pub enum $key {
            $($t($t),)*
        }

        $(
        impl From<$t> for $key {
            fn from(v: $t) -> Self {
                Self::$t(v)
            }
        }
        )*


        impl $crate::Producer for $key {
            type Output = $crate::AnyOutput;
            fn produce(&self, ctx: &$crate::QueryContext) -> anyhow::Result<Self::Output> {
                Ok(match self { $(
                    Self::$t(v) => $crate::AnyOutput::new(v.produce(ctx)?),
                )* })
            }
        }
    }
}

query_key!(QueryKey {
    ReadFile,
    ListDirectory,
});
