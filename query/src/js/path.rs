use std::path::PathBuf;

use boa_engine::{Context, JsError, JsNativeError, JsResult, value::TryFromJs};
use relative_path::RelativePath;
use relative_path::RelativePathBuf;

// SHOULD be called from a Javascript callback
fn get_current_file(js_ctx: &mut Context) -> JsResult<PathBuf> {
    Ok(js_ctx
        .get_active_script_or_module()
        .ok_or_else(|| {
            JsNativeError::eval().with_message("must be run from within a script/module")
        })?
        .path()
        .ok_or_else(|| JsNativeError::eval().with_message("script/module doesn't have filename"))?
        .to_path_buf())
}

/// Helper struct that parses a path relative to the current file itno a path relative to the cwd.
pub struct JsPath(pub PathBuf);

impl TryFromJs for JsPath {
    fn try_from_js(value: &boa_engine::JsValue, js_ctx: &mut Context) -> JsResult<Self> {
        let path = value
            .as_string()
            .ok_or_else(|| JsNativeError::typ().with_message("path must be string"))?
            .to_std_string()
            .map_err(JsError::from_rust)?;
        let base_file = get_current_file(js_ctx)?;
        let base_directory = base_file
            .parent()
            .ok_or_else(|| JsNativeError::eval().with_message("no parent directory??"))?;

        Ok(JsPath(
            RelativePathBuf::from_path(base_directory)
                .map_err(JsError::from_rust)?
                .join_normalized(RelativePath::new(&path))
                .to_path("."),
        ))
    }
}
