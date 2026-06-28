//! Collects _all_ the query keys needed to provide the wasm host functionality into one place.

use driver_query_fs::{ListDirectory, ReadFile};
use driver_query_hyper::GetUrl;

use crate::wasmtime::RunWasm;

driver_engine::query!(
QueryKey {
    ReadFile,
    ListDirectory,
    GetUrl,
    RunWasm,
} with QueryOutput);

pub type QueryContext = driver_engine::Context<QueryKey>;
