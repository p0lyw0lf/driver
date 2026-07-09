use std::cmp::Ordering;
use std::collections::btree_set::Iter;
use std::collections::{BTreeSet, HashMap};
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
        outputs: BTreeSet<WriteOutput>,
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
                    outputs: [old_self, output].into_iter().collect(),
                };
            }
            WriteOutput::Many { outputs, hash } => {
                output.hash_update(hash);
                outputs.insert(output);
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

/// Tree-traversing iterator
pub enum PathBufBlobIterator<'a> {
    Finished,
    Single((&'a PathBuf, &'a Blob)),
    More {
        curr: Iter<'a, WriteOutput>,
        prev: Option<Box<PathBufBlobIterator<'a>>>,
    },
}

impl<'a> Iterator for PathBufBlobIterator<'a> {
    type Item = (&'a PathBuf, &'a Blob);

    fn next(&mut self) -> Option<Self::Item> {
        match std::mem::replace(self, Self::Finished) {
            Self::Finished => None,
            Self::Single(item) => Some(item),
            Self::More { mut curr, prev } => loop {
                match curr.next() {
                    Some(WriteOutput::Zero) => continue,
                    Some(WriteOutput::One { path, blob }) => {
                        *self = PathBufBlobIterator::More { curr, prev };
                        return Some((path, blob));
                    }
                    Some(WriteOutput::Many { outputs, hash: _ }) => {
                        *self = PathBufBlobIterator::More {
                            curr: outputs.iter(),
                            prev: Some(Box::new(std::mem::replace(self, Self::Finished))),
                        };
                        return self.next();
                    }
                    None => {
                        *self = match prev {
                            Some(prev) => *prev,
                            None => Self::Finished,
                        };
                        return self.next();
                    }
                }
            },
        }
    }
}

impl WriteOutput {
    pub fn iter(&self) -> PathBufBlobIterator<'_> {
        match self {
            WriteOutput::Zero => PathBufBlobIterator::Finished,
            WriteOutput::One { path, blob } => PathBufBlobIterator::Single((path, blob)),
            WriteOutput::Many { outputs, hash: _ } => PathBufBlobIterator::More {
                curr: outputs.iter(),
                prev: None,
            },
        }
    }
}

enum WriteOutputIterator<'a> {
    Finished,
    Single(&'a WriteOutput),
    Many(Iter<'a, WriteOutput>),
}

impl<'a> Iterator for WriteOutputIterator<'a> {
    type Item = &'a WriteOutput;

    fn next(&mut self) -> Option<Self::Item> {
        match std::mem::replace(self, WriteOutputIterator::Finished) {
            WriteOutputIterator::Finished => None,
            WriteOutputIterator::Single(write_output) => Some(write_output),
            WriteOutputIterator::Many(mut iter) => {
                let out = iter.next();
                if out.is_some() {
                    *self = WriteOutputIterator::Many(iter);
                }
                out
            }
        }
    }
}

impl WriteOutput {
    fn iter_raw(&self) -> WriteOutputIterator<'_> {
        match self {
            WriteOutput::Zero => WriteOutputIterator::Finished,
            WriteOutput::One { .. } => WriteOutputIterator::Single(self),
            WriteOutput::Many { outputs, hash: _ } => WriteOutputIterator::Many(outputs.iter()),
        }
    }
}

#[derive(Default)]
pub struct WriteOutputDiff<'a> {
    pub to_write: HashMap<&'a PathBuf, &'a Blob>,
    pub to_remove: Vec<&'a PathBuf>,
}

impl WriteOutput {
    /// to_write will contain all the paths in [`self`] that aren't present in [`old`], and
    /// to_remove will contain all the paths in [`old`] that aren't present in [`self`].
    ///
    /// NOTE: both of those are strictly conservative estimates: to_remove might overlap with
    /// to_write. In such a case, to_write takes precedence.
    pub fn diff<'a>(&'a self, old: &'a WriteOutput) -> WriteOutputDiff<'a> {
        // TODO: these need to be iterators of raw WriteOutput, instead of iterators of (PathBuf,
        // Blob) in order for my thing to work properly.
        let mut new = self.iter_raw().peekable();
        let mut old = old.iter_raw().peekable();

        let mut to_write = HashMap::new();
        let mut to_remove = Vec::new();

        // This algorithm makes use of the property that both the new and old lists MUST be in
        // sorted order. It compares the next element in order, comparing them.
        // If the "new" one is greater, that means there are some "old" things that are missing.
        // If the "old" one is greater, that means there are some "new" things that have been added.
        // If they are equal, we can advance both.
        loop {
            let (new_output, old_output) = match (new.peek(), old.peek()) {
                (None, None) => break,
                (None, Some(old_output)) => {
                    to_remove.extend(old_output.iter().map(|(path, _)| path));
                    let _ = old.next();
                    continue;
                }
                (Some(new_output), None) => {
                    to_write.extend(new_output.iter());
                    let _ = new.next();
                    continue;
                }
                (Some(new_output), Some(old_output)) => (new_output, old_output),
            };

            match new_output.cmp(old_output) {
                Ordering::Less => {
                    to_write.extend(new_output.iter());
                    let _ = new.next();
                }
                Ordering::Greater => {
                    to_remove.extend(old_output.iter().map(|(path, _)| path));
                    let _ = old.next();
                }
                Ordering::Equal => {
                    let _ = new.next();
                    let _ = old.next();
                }
            }

            // TODO: I think this will work, but it's not recursive enough for me. That is, we no
            // longer have a way to correlate different WriteOutput::Many with each other, so we
            // can't diff ones that _should_ have some structural sharing. I'll need to think a bit
            // harder about how it's possible to do this...
        }

        WriteOutputDiff {
            to_write,
            to_remove,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn o(i: u8) -> crate::Blob {
        unsafe { crate::Blob::from_hash([i; 32].into()) }
    }

    #[test]
    fn nested_iter() {
        let mut a = WriteOutput::default();
        let a1 = (PathBuf::from("a1"), o(1));
        a.push(a1.0.clone(), a1.1.clone());
        let a2 = (PathBuf::from("a2"), o(2));
        a.push(a2.0.clone(), a2.1.clone());

        let mut b = WriteOutput::default();
        let b1 = (PathBuf::from("b1"), o(3));
        b.push(b1.0.clone(), b1.1.clone());
        let b2 = (PathBuf::from("b2"), o(4));
        b.push(b2.0.clone(), b2.1.clone());
        let b3 = (PathBuf::from("b3"), o(5));
        b.push(b3.0.clone(), b3.1.clone());

        let mut c = WriteOutput::default();
        let c1 = (PathBuf::from("c1"), o(6));
        c.push(c1.0.clone(), c1.1.clone());
        c.merge(a);
        let c2 = (PathBuf::from("c2"), o(7));
        c.push(c2.0.clone(), c2.1.clone());
        c.merge(b);
        let c3 = (PathBuf::from("c3"), o(8));
        c.push(c3.0.clone(), c3.1.clone());

        assert_eq!(
            c.iter()
                .map(|(path, blob)| (path.clone(), blob.clone()))
                .collect::<Vec<_>>(),
            [c1, a1, a2, c2, b1, b2, b3, c3,]
        );
    }
}
