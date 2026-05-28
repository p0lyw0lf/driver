use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Options {
    pub cache_path: PathBuf,
    pub remotes_path: PathBuf,
    pub objects_path: PathBuf,
}

impl Options {
    pub fn with_base_dir(dir: &Path) -> Self {
        Self {
            cache_path: dir.join("cache.zst"),
            remotes_path: dir.join("remotes.zst"),
            objects_path: dir.to_path_buf(),
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::with_base_dir(&PathBuf::from("./.driver"))
    }
}
