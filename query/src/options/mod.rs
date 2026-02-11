use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::RwLock;

#[derive(Default)]
pub struct Options {
    pub output_dir: PathBuf,
    pub cache_dir: PathBuf,
}

pub static OPTIONS: LazyLock<RwLock<Options>> = LazyLock::new(|| {
    RwLock::new(Options {
        output_dir: PathBuf::from("./dist"),
        cache_dir: PathBuf::from("./.driver_cache"),
    })
});
