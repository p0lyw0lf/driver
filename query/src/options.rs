use std::path::PathBuf;

#[derive(Debug)]
pub struct Options {
    pub output_path: PathBuf,
    pub cache_path: PathBuf,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            output_path: PathBuf::from("./dist"),
            cache_path: PathBuf::from("./.driver_cache.zst"),
        }
    }
}
