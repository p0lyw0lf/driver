use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use driver_engine::Object;

/// Turns command-line arguments into a javascript-compatible list.
/// TODO: better types than `&str`.
pub fn parse_args<'a>(iter: impl IntoIterator<Item = &'a str>) -> () {
    todo!("parse them into the expected wasm value type")
}

pub type WriteOutputs = BTreeMap<PathBuf, Object>;

driver_engine::key!(
    #[input=|_| false]
    struct RunWasm {
        pub file: PathBuf,
        pub num: u8,
        pub arg: (),
    }
);

driver_engine::object_trace!(RunWasm => { arg });

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunWasmOutput {
    pub export: (),
    pub writes: WriteOutputs,
}
driver_engine::object_trace!(RunWasmOutput => {
    export,
    writes,
});

driver_engine::producer!(RunWasm(self, ctx) as (crate::QueryKey) -> driver_util::Result<RunWasmOutput> {
    todo!()
});

impl std::fmt::Display for RunWasm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "run(\"{}\", {}, {})",
            self.file.display(),
            self.num,
            todo!("self.arg")
        )
    }
}
