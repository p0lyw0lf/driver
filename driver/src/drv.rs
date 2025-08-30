use std::fs::File;
use std::fs::create_dir_all;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use bincode::Decode;
use bincode::Encode;
use nix_base32::to_nix_base32;
use sha2::Digest;
use sha2::Sha256;

#[derive(Encode, Decode)]
pub struct Derivation {
    /// An executable program w/ arguments to run a build. The output MUST be printed to stdout.
    pub builder: Vec<String>,
    /// The path this derivation will output into, relative to the output directory.
    pub output_path: String,
}

impl Derivation {
    pub fn output_path(&self) -> Box<Path> {
        let bytes =
            bincode::encode_to_vec(self, bincode::config::standard()).expect("encoding error");
        let digest = Sha256::digest(&bytes);
        let mut output_path = PathBuf::new();
        output_path.push("cache");
        output_path.push(to_nix_base32(&digest));
        // TODO: prevent path traversal
        output_path.push(&self.output_path);

        output_path.into_boxed_path()
    }

    pub fn run(&self) -> std::io::Result<()> {
        let output_path = self.output_path();
        create_dir_all(output_path.parent().unwrap())?;
        let output = std::process::Command::new(&self.builder[0])
            .args(&self.builder[1..])
            .output()?;

        if !output.status.success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("process failed with exit code {}", output.status),
            ));
        }

        let mut file = File::create(&output_path)?;
        file.write_all(&output.stdout)?;

        Ok(())
    }
}
