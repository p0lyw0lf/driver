use std::collections::{BTreeMap, btree_map};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::blob::{Blob, BlobTrace};
use crate::hash::{Hash, Sha256Hasher};

#[derive(Debug, Serialize, Deserialize)]
struct PartialWriteOutput<Key: Ord> {
    direct: BTreeMap<PathBuf, Blob>,
    // TODO: Right now, the main complaint I have is that all these indirect dependencies are full
    // objects, meaning they need to be cloned fresh out of the cached computation store every time.
    // There's **gotta* be a way to get around this, but I haven't found it yet...
    //
    // All this does so far is do a slightly (algorithmically) faster diffing algorithm that can
    // ignore when subkeys haven't changed. What we can't do (and what I really really want to do)
    // is make is so that _we don't even need to materialize dirs that haven't changed_.
    //
    // I think the way we need to do this is with another content-addressed sore? But just for the
    // WriteOutputs instead of the Blobs.
    indirect: BTreeMap<Key, WriteOutput<Key>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WriteOutput<Key: Ord> {
    inner: PartialWriteOutput<Key>,
    /// Memoized hash of [`inner`] to make comparisons faster.
    hash: Hash,
}

/// We want this clone impl, but we _don't_ want a clone impl for [`WriteOutputBuilder`], so we have
/// to write it manually.
impl<Key: Ord + Clone> Clone for WriteOutput<Key> {
    fn clone(&self) -> Self {
        Self {
            inner: PartialWriteOutput {
                direct: self.inner.direct.clone(),
                indirect: self.inner.indirect.clone(),
            },
            hash: self.hash,
        }
    }
}

pub type WriteOutputBuilder<Key> = PartialWriteOutput<Key>;

impl<Key: Ord + std::hash::Hash> WriteOutputBuilder<Key> {
    pub fn new() -> Self {
        Self {
            direct: BTreeMap::new(),
            indirect: BTreeMap::new(),
        }
    }

    pub fn push(&mut self, path: PathBuf, blob: Blob) {
        // TODO: as an optimization, if we ever get "too many" direct things (say like 32? or 128?)
        // we should be able to put the current PartialWriteOutput as an indirect dependency
        // instead. This requires content-addressing all the WriteOutputs somehow but I think that
        // should be doable?
        let _ = self.direct.insert(path, blob);
    }

    pub fn merge(&mut self, key: Key, output: WriteOutput<Key>) {
        let _ = self.indirect.insert(key, output);
    }

    pub fn finalize(self) -> WriteOutput<Key> {
        let mut hasher = Sha256Hasher::new();

        for (path, blob) in self.direct.iter() {
            hasher.update(path.as_os_str().as_encoded_bytes());
            hasher.update(blob);
        }

        for (key, output) in self.indirect.iter() {
            key.hash(&mut hasher);
            hasher.update(output.hash);
        }

        let hash = hasher.finalize();

        WriteOutput { inner: self, hash }
    }
}

impl<Key: Ord> BlobTrace for WriteOutput<Key> {
    fn trace(&self) -> impl Iterator<Item = &'_ Blob> {
        self.iter().map(|(_path, blob)| blob)
    }
}

pub struct WriteOutputIter<'a, Key: Ord> {
    /// The current level of `direct` items we're iterating over.
    direct: btree_map::Iter<'a, PathBuf, Blob>,
    /// The current level of `indirect` items we're iterating over.
    indirect: btree_map::Values<'a, Key, WriteOutput<Key>>,
    /// The level we came from, if applicable.
    parent: Option<Box<WriteOutputIter<'a, Key>>>,
}

impl<'a, Key: Ord> Iterator for WriteOutputIter<'a, Key> {
    type Item = (&'a PathBuf, &'a Blob);

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.direct.next() {
            return Some(item);
        }

        match self.indirect.next() {
            Some(next_level) => {
                let mut child = next_level.iter();
                std::mem::swap(self, &mut child);
                self.parent = Some(Box::new(child));

                self.next()
            }
            None => match self.parent.take() {
                Some(parent) => {
                    *self = *parent;
                    self.next()
                }
                None => None,
            },
        }
    }
}

impl<Key: Ord> WriteOutput<Key> {
    pub fn iter(&self) -> WriteOutputIter<'_, Key> {
        WriteOutputIter {
            direct: self.inner.direct.iter(),
            indirect: self.inner.indirect.values(),
            parent: None,
        }
    }
}

/*
#[derive(Default)]
pub struct WriteOutputDiff<'a> {
    pub to_write: HashMap<&'a PathBuf, &'a Blob>,
    pub to_remove: Vec<&'a PathBuf>,
}

impl<Key: Ord> WriteOutput<Key> {
    /// to_write will contain all the paths in [`self`] that aren't present in [`old`], and
    /// to_remove will contain all the paths in [`old`] that aren't present in [`self`].
    ///
    /// NOTE: both of those are strictly conservative estimates: to_remove might overlap with
    /// to_write. In such a case, to_write takes precedence.
    pub fn diff<'a>(&'a self, old: &'a WriteOutput<Key>) -> WriteOutputDiff<'a> {
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
*/

#[cfg(test)]
mod test {
    use super::*;

    fn o(i: u8) -> crate::Blob {
        unsafe { crate::Blob::from_hash([i; 32].into()) }
    }

    #[test]
    fn nested_iter() {
        let mut a = WriteOutputBuilder::new();
        let a1 = (PathBuf::from("a1"), o(1));
        a.push(a1.0.clone(), a1.1.clone());
        let a2 = (PathBuf::from("a2"), o(2));
        a.push(a2.0.clone(), a2.1.clone());
        let a = a.finalize();

        let mut b = WriteOutputBuilder::new();
        let b1 = (PathBuf::from("b1"), o(3));
        b.push(b1.0.clone(), b1.1.clone());
        let b2 = (PathBuf::from("b2"), o(4));
        b.push(b2.0.clone(), b2.1.clone());
        let b3 = (PathBuf::from("b3"), o(5));
        b.push(b3.0.clone(), b3.1.clone());
        let b = b.finalize();

        let mut c = WriteOutputBuilder::new();
        let c1 = (PathBuf::from("c1"), o(6));
        c.push(c1.0.clone(), c1.1.clone());
        c.merge("a".to_string(), a);
        let c2 = (PathBuf::from("c2"), o(7));
        c.push(c2.0.clone(), c2.1.clone());
        c.merge("b".to_string(), b);
        let c3 = (PathBuf::from("c3"), o(8));
        c.push(c3.0.clone(), c3.1.clone());
        let c = c.finalize();

        assert_eq!(
            c.iter()
                .map(|(path, blob)| (path.clone(), blob.clone()))
                .collect::<Vec<_>>(),
            [c1, c2, c3, a1, a2, b1, b2, b3]
        );
    }
}
