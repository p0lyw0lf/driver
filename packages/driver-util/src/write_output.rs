use std::cmp::Ordering;
use std::path::PathBuf;

use crypto_common::hazmat::{SerializableState, SerializedState};
use crypto_common::typenum::Unsigned;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::Blob;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum WriteOutput {
    /// Represents no writes. Exists so we don't _always_ allocate a Vec.
    #[default]
    Zero,
    /// Represents a single desired write.
    One { path: PathBuf, blob: Blob },
    /// Represents a collection of writes.
    Many {
        outputs: Vec<WriteOutput>,
        /// Collected hash of every node inside [`outputs`], for faster comparisons.
        /// We need to be extremely cautious in upholding this guarantee; in particular, due to the
        /// way hashes are calculated, we MUST NOT modify children added to the [`outputs`] after
        /// they've been added. The API we expose should be sufficient for this.
        #[serde(serialize_with = "serialize_hash")]
        #[serde(deserialize_with = "deserialize_hash")]
        hash: sha2::Sha256,
    },
}
crate::no_blobs!(WriteOutput);

impl WriteOutput {
    fn hash_update(&self, hash: &mut impl Digest) {
        match self {
            WriteOutput::Zero => {}
            WriteOutput::One { path, blob } => {
                hash.update(b"One");
                hash.update(path.as_os_str().as_encoded_bytes());
                hash.update(blob);
            }
            WriteOutput::Many {
                outputs: _,
                hash: sub_hash,
            } => {
                hash.update(b"Many");
                hash.update(sub_hash.clone().finalize());
            }
        }
    }

    pub fn push(&mut self, path: PathBuf, blob: Blob) {
        self.merge(WriteOutput::One { path, blob });
    }

    pub fn merge(&mut self, output: WriteOutput) {
        match self {
            WriteOutput::Zero => *self = output,
            WriteOutput::One { .. } => {
                let old_self = std::mem::take(self);
                *self = WriteOutput::Many {
                    hash: {
                        let mut hash = sha2::Sha256::default();
                        old_self.hash_update(&mut hash);
                        output.hash_update(&mut hash);
                        hash
                    },
                    outputs: vec![old_self, output],
                };
            }
            WriteOutput::Many { outputs, hash } => {
                output.hash_update(hash);
                outputs.push(output);
            }
        }
    }
}

impl PartialEq for WriteOutput {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Zero, Self::Zero) => true,
            (
                Self::One {
                    path: l_path,
                    blob: l_blob,
                },
                Self::One {
                    path: r_path,
                    blob: r_blob,
                },
            ) => l_path == r_path && l_blob == r_blob,
            (
                Self::Many {
                    outputs: _,
                    hash: l_hash,
                },
                Self::Many {
                    outputs: _,
                    hash: r_hash,
                },
            ) => l_hash.clone().finalize() == r_hash.clone().finalize(),
            _ => false,
        }
    }
}
impl Eq for WriteOutput {}

impl Ord for WriteOutput {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Zero, Self::Zero) => Ordering::Equal,
            (
                Self::One {
                    path: l_path,
                    blob: l_blob,
                },
                Self::One {
                    path: r_path,
                    blob: r_blob,
                },
            ) => l_path.cmp(r_path).then(l_blob.cmp(r_blob)),
            (
                Self::Many {
                    outputs: _,
                    hash: l_hash,
                },
                Self::Many {
                    outputs: _,
                    hash: r_hash,
                },
            ) => l_hash.clone().finalize().cmp(&r_hash.clone().finalize()),
            (Self::Zero, Self::One { .. }) => Ordering::Less,
            (Self::Zero, Self::Many { .. }) => Ordering::Less,
            (Self::One { .. }, Self::Zero) => Ordering::Greater,
            (Self::One { .. }, Self::Many { .. }) => Ordering::Less,
            (Self::Many { .. }, Self::Zero) => Ordering::Greater,
            (Self::Many { .. }, Self::One { .. }) => Ordering::Greater,
        }
    }
}

impl PartialOrd for WriteOutput {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::hash::Hash for WriteOutput {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            WriteOutput::Zero => {}
            WriteOutput::One { path, blob } => {
                path.hash(state);
                blob.hash(state);
            }
            WriteOutput::Many { outputs: _, hash } => {
                hash.clone().finalize().hash(state);
            }
        }
    }
}

fn serialize_hash<S>(hash: &sha2::Sha256, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::ser::Serializer,
{
    serializer.serialize_bytes(SerializableState::serialize(hash).as_ref())
}

fn deserialize_hash<'de, D>(deserializer: D) -> Result<sha2::Sha256, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    struct HashVisitor;

    impl<'de> serde::de::Visitor<'de> for HashVisitor {
        type Value = sha2::Sha256;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(
                f,
                "expecting sha256 hasher state of {} bytes",
                <sha2::Sha256 as SerializableState>::SerializedStateSize::to_usize()
            )
        }

        fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            let state = SerializedState::<sha2::Sha256>::try_from(v)
                .map_err(|_| serde::de::Error::invalid_length(v.len(), &HashVisitor))?;
            SerializableState::deserialize(&state)
                .map_err(|_| serde::de::Error::custom("could not restore from state"))
        }
    }

    deserializer.deserialize_bytes(HashVisitor)
}

enum WriteOutputIterator<'a> {
    Finished,
    More {
        curr: std::slice::Iter<'a, WriteOutput>,
        prev: Option<&'a WriteOutput>,
    },
}
