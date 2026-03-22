use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::RwLock;

#[derive(Default)]
pub struct Options {
    pub output_path: PathBuf,
    pub cache_path: PathBuf,
}

pub static OPTIONS: LazyLock<RwLock<Options>> = LazyLock::new(|| {
    RwLock::new(Options {
        output_path: PathBuf::from("./dist"),
        cache_path: PathBuf::from("./.driver_cache.zst"),
    })
});
