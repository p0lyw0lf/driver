use std::fs::create_dir_all;
use std::path::Path;
use std::path::PathBuf;

use bincode::Decode;
use bincode::Encode;
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
    /// The path of the file to copy
    pub input_path: String,
    /// The expected hash of the file
    pub digest: Vec<u8>,
}

impl Derivation for FileDerivation {
    fn run(&self) -> std::io::Result<()> {
        let bytes = std::fs::read(&self.input_path)?;
        let digest = Sha256::digest(&bytes);

        if digest.as_slice() != self.digest {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    r#"
expected {} to have hash
    {}, got hash
    {}
"#,
                    self.input_path,
                    to_nix_base32(&self.digest),
                    to_nix_base32(&digest)
                ),
            ));
        }

        let mut output_path = self.output_path();
        create_dir_all(&output_path)?;
        let input_path = Path::new(&self.input_path).file_name().unwrap();
        output_path.push(input_path);

        std::fs::write(&output_path, bytes)?;

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
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "process failed with exit code {}:\n{}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr),
                ),
            ));
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
