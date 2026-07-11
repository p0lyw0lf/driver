use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet, btree_map};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::blob::{Blob, BlobTrace};
use crate::hash::{Hash, Sha256Hasher};

/// To construct this, use [`WriteOutput::builder()`].
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriteOutputBuilder<Key: Ord> {
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

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriteOutput<Key: Ord> {
    inner: WriteOutputBuilder<Key>,
    /// Memoized hash of [`inner`] to make comparisons faster.
    hash: Hash,
}

/// We want this clone impl, but we _don't_ want a clone impl for [`WriteOutputBuilder`], so we have
/// to write it manually.
impl<Key: Ord + Clone> Clone for WriteOutput<Key> {
    fn clone(&self) -> Self {
        Self {
            inner: WriteOutputBuilder {
                direct: self.inner.direct.clone(),
                indirect: self.inner.indirect.clone(),
            },
            hash: self.hash,
        }
    }
}

impl<Key: Ord + std::hash::Hash> WriteOutputBuilder<Key> {
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
    pub fn builder() -> WriteOutputBuilder<Key> {
        WriteOutputBuilder {
            direct: BTreeMap::new(),
            indirect: BTreeMap::new(),
        }
    }

    pub fn iter(&self) -> WriteOutputIter<'_, Key> {
        WriteOutputIter {
            direct: self.inner.direct.iter(),
            indirect: self.inner.indirect.values(),
            parent: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct WriteOutputDiff<'a> {
    pub to_write: HashMap<&'a PathBuf, &'a Blob>,
    pub to_remove: HashSet<&'a PathBuf>,
}

impl<'a> WriteOutputDiff<'a> {
    fn write(&mut self, elem: &(&'a PathBuf, &'a Blob)) {
        self.to_write.insert(elem.0, elem.1);
        self.to_remove.remove(elem.0);
    }

    fn remove(&mut self, elem: &(&'a PathBuf, &'a Blob)) {
        if !self.to_write.contains_key(elem.0) {
            self.to_remove.insert(elem.0);
        }
    }

    pub fn diff<Key: Ord>(new: &'a WriteOutput<Key>, old: &'a WriteOutput<Key>) -> Self {
        let mut diff = Self::default();
        diff.add_from(new, old);
        diff
    }

    fn add_from<Key: Ord>(&mut self, new: &'a WriteOutput<Key>, old: &'a WriteOutput<Key>) {
        let mut new_direct = new.inner.direct.iter().peekable();
        let mut old_direct = old.inner.direct.iter().peekable();

        // This algorithm makes use of the property that both the new and old lists MUST be in
        // sorted order. It compares the next element in order, comparing them.
        // If the "new" one is greater, that means there are some "old" things that are missing.
        // If the "old" one is greater, that means there are some "new" things that have been added.
        // If they are equal, we can advance both.
        //
        // First, we do this sort of iteration over all the same direct dependencies for this node.
        loop {
            let (new_output, old_output) = match (new_direct.peek(), old_direct.peek()) {
                (None, None) => break,
                (None, Some(old_output)) => {
                    self.remove(old_output);
                    let _ = old_direct.next();
                    continue;
                }
                (Some(new_output), None) => {
                    self.write(new_output);
                    let _ = new_direct.next();
                    continue;
                }
                (Some(new_output), Some(old_output)) => (new_output, old_output),
            };

            match new_output.0.cmp(old_output.0) {
                Ordering::Less => {
                    self.write(new_output);
                    let _ = new_direct.next();
                }
                Ordering::Greater => {
                    self.remove(old_output);
                    let _ = old_direct.next();
                }
                Ordering::Equal => {
                    if new_output.1 != old_output.1 {
                        self.write(new_output);
                    }
                    let _ = new_direct.next();
                    let _ = old_direct.next();
                }
            }
        }

        // Next, do this sort of iteration for all the indirect dependencies. The efficiency of this
        // algorithm relies on indirect dependencies "likely not having changed" and no keys moving
        // between them.

        let mut new_indirect = new.inner.indirect.iter().peekable();
        let mut old_indirect = old.inner.indirect.iter().peekable();

        loop {
            let (new_output, old_output) = match (new_indirect.peek(), old_indirect.peek()) {
                (None, None) => break,
                (None, Some(old_output)) => {
                    for elem in old_output.1.iter() {
                        self.remove(&elem);
                    }
                    let _ = old_indirect.next();
                    continue;
                }
                (Some(new_output), None) => {
                    for elem in new_output.1.iter() {
                        self.write(&elem);
                    }
                    let _ = new_indirect.next();
                    continue;
                }
                (Some(new_output), Some(old_output)) => (new_output, old_output),
            };

            match new_output.0.cmp(old_output.0) {
                Ordering::Less => {
                    for elem in new_output.1.iter() {
                        self.write(&elem);
                    }
                    let _ = new_indirect.next();
                }
                Ordering::Greater => {
                    for elem in old_output.1.iter() {
                        self.remove(&elem);
                    }
                    let _ = old_indirect.next();
                }
                Ordering::Equal => {
                    if new_output.1.hash != old_output.1.hash {
                        self.add_from(new_output.1, old_output.1);
                    }
                    let _ = new_indirect.next();
                    let _ = old_indirect.next();
                }
            }
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
        let mut a = WriteOutput::builder();
        let a1 = (PathBuf::from("a1"), o(1));
        a.push(a1.0.clone(), a1.1.clone());
        let a2 = (PathBuf::from("a2"), o(2));
        a.push(a2.0.clone(), a2.1.clone());
        let a = a.finalize();

        let mut b = WriteOutput::builder();
        let b1 = (PathBuf::from("b1"), o(3));
        b.push(b1.0.clone(), b1.1.clone());
        let b2 = (PathBuf::from("b2"), o(4));
        b.push(b2.0.clone(), b2.1.clone());
        let b3 = (PathBuf::from("b3"), o(5));
        b.push(b3.0.clone(), b3.1.clone());
        let b = b.finalize();

        let mut c = WriteOutput::builder();
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

    #[test]
    fn nested_diff() {
        let tree1 = {
            let mut a = WriteOutput::builder();
            let a1 = (PathBuf::from("a1"), o(1));
            a.push(a1.0.clone(), a1.1.clone());
            let a2 = (PathBuf::from("a2"), o(2));
            a.push(a2.0.clone(), a2.1.clone());
            let a = a.finalize();

            let mut b = WriteOutput::builder();
            let b1 = (PathBuf::from("b1"), o(3));
            b.push(b1.0.clone(), b1.1.clone());
            let b2 = (PathBuf::from("b2"), o(4));
            b.push(b2.0.clone(), b2.1.clone());
            let b3 = (PathBuf::from("b3"), o(5));
            b.push(b3.0.clone(), b3.1.clone());
            let b = b.finalize();

            let mut c = WriteOutput::builder();
            let c1 = (PathBuf::from("c1"), o(6));
            c.push(c1.0.clone(), c1.1.clone());
            let c2 = (PathBuf::from("c2"), o(7));
            c.push(c2.0.clone(), c2.1.clone());
            let c3 = (PathBuf::from("c3"), o(8));
            c.push(c3.0.clone(), c3.1.clone());
            let c = c.finalize();

            let mut d = WriteOutput::builder();
            d.merge("a".to_string(), a);
            d.merge("b".to_string(), b);
            d.merge("c".to_string(), c);
            d.finalize()
        };

        let tree2 = {
            let mut a = WriteOutput::builder();
            let a1 = (PathBuf::from("a1"), o(1));
            a.push(a1.0.clone(), a1.1.clone());
            let a2 = (PathBuf::from("a2"), o(2));
            a.push(a2.0.clone(), a2.1.clone());
            // CHANGE: extra element in a
            let a3 = (PathBuf::from("a3"), o(3));
            a.push(a3.0.clone(), a3.1.clone());
            let a = a.finalize();

            let mut b = WriteOutput::builder();
            // CHANGE: one less element in b
            // let b1 = (PathBuf::from("b1"), o(3));
            // b.push(b1.0.clone(), b1.1.clone());
            let b2 = (PathBuf::from("b2"), o(4));
            b.push(b2.0.clone(), b2.1.clone());
            let b3 = (PathBuf::from("b3"), o(5));
            b.push(b3.0.clone(), b3.1.clone());
            let b = b.finalize();

            let mut c = WriteOutput::builder();
            let c1 = (PathBuf::from("c1"), o(6));
            c.push(c1.0.clone(), c1.1.clone());
            // CHANGE: different element in c
            let c2 = (PathBuf::from("c2"), o(9));
            c.push(c2.0.clone(), c2.1.clone());
            let c3 = (PathBuf::from("c3"), o(8));
            c.push(c3.0.clone(), c3.1.clone());
            let c = c.finalize();

            let mut d = WriteOutput::builder();
            d.merge("a".to_string(), a);
            d.merge("b".to_string(), b);
            d.merge("c".to_string(), c);
            d.finalize()
        };

        let diff = WriteOutputDiff::diff(&tree2, &tree1);

        assert_eq!(
            diff.to_write,
            [(&PathBuf::from("a3"), &o(3)), (&PathBuf::from("c2"), &o(9))]
                .into_iter()
                .collect()
        );
        assert_eq!(diff.to_remove, [&PathBuf::from("b1")].into_iter().collect());
    }
}
