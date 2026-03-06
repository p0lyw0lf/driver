use std::path::PathBuf;

use relative_path::RelativePath;
use relative_path::RelativePathBuf;
use rquickjs::Ctx;
use rquickjs::FromJs;

// SHOULD be called from a Javascript callback
fn get_current_file(js_ctx: &Ctx<'_>) -> rquickjs::Result<PathBuf> {
    Ok(PathBuf::from(
        js_ctx
            .script_or_module_name(0)
            .ok_or(super::error_message("not running in a module"))?
            .to_string()?,
    ))
}

/// Helper struct that parses a path relative to the current file itno a path relative to the cwd.
pub struct JsPath(pub PathBuf);

impl<'js> FromJs<'js> for JsPath {
    fn from_js(js_ctx: &Ctx<'js>, value: rquickjs::Value<'js>) -> rquickjs::Result<Self> {
        let path = value
            .as_string()
            .ok_or_else(|| rquickjs::Error::new_from_js(value.type_name(), "PathBuf"))?
            .to_string()?;
        let base_file = get_current_file(js_ctx)?;
        let base_directory = base_file
            .parent()
            .ok_or(super::error_message("no parent directory?"))?;

        Ok(JsPath(
            RelativePathBuf::from_path(base_directory)
                .map_err(|e| {
                    rquickjs::Error::new_from_js_message("String", "RelativePath", e.to_string())
                })?
                .join_normalized(RelativePath::new(&path))
                .to_path("."),
        ))
    }
}
