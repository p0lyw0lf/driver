use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use ignore::gitignore::{GitignoreBuilder, gitconfig_excludes_path};
use sha2::Digest;
use sha2::Sha256;

type Sha256Digest = sha2::digest::Output<sha2::Sha256>;

/// Recursively walks a directory, hashing files as it goes, to produce
pub fn walk(dir: PathBuf) -> anyhow::Result<Sha256Digest> {
    let mut builder = GitignoreBuilder::new(&dir);
    if let Some(excludes) = gitconfig_excludes_path()
        && excludes.exists()
    {
        add_path(&mut builder, &excludes)
            .with_context(|| format!("failed to add {}", excludes.display()))?;
    }
    builder.add_line(None, ".git/")?;
    walk_with_ignores(dir, builder)
}

fn add_path(builder: &mut GitignoreBuilder, path: impl AsRef<Path>) -> anyhow::Result<()> {
    match builder.add(path) {
        None => Ok(()),
        Some(err) => Err(anyhow::Error::new(err)),
    }
}

fn walk_with_ignores(dir: PathBuf, mut builder: GitignoreBuilder) -> anyhow::Result<Sha256Digest> {
    let mut gitignore_path = dir.clone();
    gitignore_path.push(".gitignore");
    if gitignore_path.exists() {
        add_path(&mut builder, &gitignore_path)
            .with_context(|| format!("failed to add {}", gitignore_path.display()))?;
    }

    let gitignore = builder.build()?;

    let mut entries = std::fs::read_dir(&dir)?.collect::<Result<Vec<_>, _>>()?;

    // Rust has no guarantees about
    entries.sort_by(|a, b| {
        a.file_name()
            .partial_cmp(&b.file_name())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut hasher = Sha256::new();

    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::metadata(&path)?;
        let is_dir = metadata.is_dir();
        if matches!(gitignore.matched(&path, is_dir), ignore::Match::Ignore(_)) {
            // Skip over anything in the .gitignore
            continue;
        }

        if is_dir {
            // println!("walking {}", path.display());
            let digest = walk_with_ignores(path, builder.clone())?;
            hasher.update(digest);
        } else {
            // println!("reading {}", path.display());
            let bytes = std::fs::read(path)?;
            hasher.update(&bytes);
        }
    }

    Ok(hasher.finalize())
}
