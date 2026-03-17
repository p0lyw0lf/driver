use std::path::PathBuf;

use boa_engine::{Context, JsError, JsNativeError, JsResult, value::TryFromJs};
use relative_path::RelativePath;
use relative_path::RelativePathBuf;

/// Helper struct that parses a path relative to the cwd of the binary (the project root).
pub struct JsPath(pub PathBuf);

impl TryFromJs for JsPath {
    fn try_from_js(value: &boa_engine::JsValue, _js_ctx: &mut Context) -> JsResult<Self> {
        let path = value
            .as_string()
            .ok_or_else(|| JsNativeError::typ().with_message("path must be string"))?
            .to_std_string()
            .map_err(JsError::from_rust)?;

        Ok(JsPath(
            RelativePathBuf::from_path(".")
                .map_err(JsError::from_rust)?
                .join_normalized(RelativePath::new(&path))
                .to_path("."),
        ))
    }
}
