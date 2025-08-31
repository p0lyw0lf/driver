use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs::create_dir_all;
use std::path::PathBuf;

use bincode::Decode;
use bincode::Encode;
use globset::Glob;
use nix_base32::to_nix_base32;
use sha2::Digest;
use sha2::Sha256;

/// A Derivation is something that can be
pub trait Derivation {
    /// The output directory for the derivation, calculated based on its hash
    fn output_path(&self) -> PathBuf
    where
        Self: Encode,
    {
        let bytes =
            bincode::encode_to_vec(self, bincode::config::standard()).expect("encoding error");
        let digest = Sha256::digest(&bytes);
        let mut output_path = PathBuf::new();
        output_path.push("cache");
        output_path.push(to_nix_base32(&digest));

        output_path
    }

    /// Running the derivation, producing files at the given output_path.
    fn run(&self) -> std::io::Result<()>;
}

/// A FileDerivation is used to reproducibly copy files to the store.
#[derive(Encode, Decode)]
pub struct FileDerivation {
    /// The file/directory to copy.
    pub input: FileInput,
    /// The expected hash of the file/directory.
    pub digest: Vec<u8>,
}

#[derive(Encode, Decode, Debug)]
pub struct FileInput {
    /// The base path of the file/directory to walk
    pub path: String,
    /// If path is a directory, the glob for all the files to walk. Defaults to "*"
    pub glob: Option<String>,
}

impl Display for FileInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path)?;
        if let Some(glob) = &self.glob {
            write!(f, ", {glob}")?;
        }

        Ok(())
    }
}

struct WalkOutput {
    digest: sha2::digest::Output<Sha256>,
    files: BTreeMap<PathBuf, Vec<u8>>,
}

impl FileInput {
    fn walk(&self) -> std::io::Result<WalkOutput> {
        let glob = match &self.glob {
            Some(glob) => Some(
                Glob::new(glob)
                    .map_err(|err| std::io::Error::other(format!("Making glob: {}", err)))?
                    .compile_matcher(),
            ),
            None => None,
        };
        let mut hasher = Sha256::new();
        let mut files = BTreeMap::new();

        for result in ignore::Walk::new(&self.path) {
            let entry =
                result.map_err(|err| std::io::Error::other(format!("walking tree: {}", err)))?;

            if entry
                .file_type()
                .is_some_and(|ty| ty.is_file() || ty.is_symlink())
                && glob.as_ref().is_none_or(|g| g.is_match(entry.path()))
            {
                let bytes = std::fs::read(entry.path())?;
                hasher.update(&bytes);

                let filename = match glob {
                    Some(_) => entry
                        .path()
                        .strip_prefix(&self.path)
                        .expect("entry did not begin with input_path")
                        .to_owned(),
                    None => entry
                        .path()
                        .file_name()
                        .expect("entry does not have filename")
                        .into(),
                };

                files.insert(filename, bytes);
            }
        }

        Ok(WalkOutput {
            digest: hasher.finalize(),
            files,
        })
    }

    pub(crate) fn digest(&self) -> std::io::Result<sha2::digest::Output<Sha256>> {
        Ok(self.walk()?.digest)
    }

    pub(crate) fn files(&self) -> std::io::Result<Vec<PathBuf>> {
        Ok(self.walk()?.files.into_keys().collect::<Vec<_>>())
    }
}

impl Derivation for FileDerivation {
    fn run(&self) -> std::io::Result<()> {
        let WalkOutput {
            digest: expected_digest,
            files,
        } = self.input.walk()?;
        if expected_digest.as_slice() != self.digest {
            return Err(std::io::Error::other(format!(
                r#"
expected {} to have hash
    {}, got hash
    {}
"#,
                self.input,
                to_nix_base32(&self.digest),
                to_nix_base32(&expected_digest)
            )));
        }

        let output_path = self.output_path();
        for (filename, bytes) in files {
            let output_filename = output_path.join(filename);
            create_dir_all(output_filename.parent().unwrap())?;
            std::fs::write(output_filename, bytes)?;
        }

        Ok(())
    }
}

/// A BuildDerivation is a way to reproducibly build some file. It's reproducible because only the
/// `stdout` of the builder is used to create the file.
#[derive(Encode, Decode)]
pub struct BuildDerivation {
    /// An executable program w/ arguments to run a build. Any argument containing the string
    /// "$out" will have that part of the string replaced with the calculated output path.
    pub builder: Vec<String>,
}

impl Derivation for BuildDerivation {
    fn run(&self) -> std::io::Result<()> {
        let output_path = self.output_path();
        create_dir_all(&output_path)?;

        let cmd = &self.builder[0];
        let output_path_str = output_path.to_str().unwrap();
        let args = self.builder[1..]
            .iter()
            .map(|arg| arg.replace("$out", output_path_str))
            .collect::<Vec<_>>();

        let output = std::process::Command::new(cmd).args(&args).output()?;

        if !output.status.success() {
            return Err(std::io::Error::other(format!(
                "process failed with exit code {}:\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr),
            )));
        }

        Ok(())
    }
}

pub enum AnyDerivation {
    File(FileDerivation),
    Build(BuildDerivation),
}

impl Derivation for AnyDerivation {
    fn output_path(&self) -> PathBuf {
        match self {
            AnyDerivation::File(f) => f.output_path(),
            AnyDerivation::Build(b) => b.output_path(),
        }
    }
    fn run(&self) -> std::io::Result<()> {
        match self {
            AnyDerivation::File(f) => f.run(),
            AnyDerivation::Build(b) => b.run(),
        }
    }
}
