use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::PathBuf;

use inotify::{WatchDescriptor, WatchMask};

/// Represents a single watch of a path.
struct Watch {
    /// The inode that we're watching.
    descriptor: WatchDescriptor,
    /// Reference count for how many different consumers are watching this path. SHOULD be non-zero
    /// (but isn't because NonZeroU32 is hard to use)
    count: u32,
}

/// Represents a set of watches that are currently active. That is, if we have an object of this
/// type, then we also have an [`inotify::Watches`] that is watching the files described here.
struct ActiveWatches {
    /// The actual watches to keep in sync.
    watches: inotify::Watches,
    /// Reference count for how many paths are watching this descriptor.
    descriptors: HashMap<WatchDescriptor, u32>,
    paths: HashMap<PathBuf, Watch>,
}

impl ActiveWatches {
    fn new(watches: inotify::Watches) -> Self {
        Self {
            watches,
            descriptors: Default::default(),
            paths: Default::default(),
        }
    }

    fn add_file(&mut self, path: PathBuf, count: u32) {
        self.paths
            .entry(path.clone())
            .and_modify(|w| w.count += count)
            .or_insert_with(|| {
                let descriptor = self
                    .watches
                    .add(
                        &path,
                        WatchMask::CLOSE_WRITE | WatchMask::DELETE_SELF | WatchMask::MOVE_SELF,
                    )
                    .expect("TODO");

                *self.descriptors.entry(descriptor.clone()).or_insert(0) += 1;
                Watch { descriptor, count }
            });
    }

    fn add_directory(&mut self, path: PathBuf, count: u32) {
        self.paths
            .entry(path.clone())
            .and_modify(|w| w.count += count)
            .or_insert_with(|| {
                let descriptor = self
                    .watches
                    .add(
                        &path,
                        WatchMask::CREATE
                            | WatchMask::DELETE
                            | WatchMask::MOVED_FROM
                            | WatchMask::MOVED_TO
                            | WatchMask::DELETE_SELF
                            | WatchMask::MOVE_SELF,
                    )
                    .expect("TODO");

                *self.descriptors.entry(descriptor.clone()).or_insert(0) += 1;
                Watch { descriptor, count }
            });
    }

    fn remove(&mut self, path: &PathBuf, count: u32) {
        let watch = match self.paths.get_mut(path) {
            Some(watch) => watch,
            None => return,
        };

        if watch.count > count {
            watch.count -= count;
            return;
        }

        if count > watch.count {
            panic!("tried to remove more times than we had {}", path.display());
        }

        // If removing the last reference, also remove the watch.
        let descriptor = self.paths.remove(path).unwrap().descriptor;
        self.remove_descriptor(descriptor);
    }

    fn remove_descriptor(&mut self, descriptor: WatchDescriptor) {
        let count = self
            .descriptors
            .get_mut(&descriptor)
            .expect("tried to get unknown descriptor");
        if *count > 1 {
            *count -= 1;
            return;
        }

        self.descriptors.remove(&descriptor).unwrap();
        self.watches.remove(descriptor).expect("TODO");
    }
}

/// Represents a potentially out-of-date set of watches. Efficiently keeps track of the diff between
/// the active watch set and the new desired watch set, which can be applied all at once.
pub struct Watches {
    active_watches: ActiveWatches,
    pending_files: HashMap<PathBuf, i32>,
    pending_directories: HashMap<PathBuf, i32>,
}

impl Watches {
    pub fn new(watches: inotify::Watches) -> Self {
        Self {
            active_watches: ActiveWatches::new(watches),
            pending_files: Default::default(),
            pending_directories: Default::default(),
        }
    }

    pub fn add_file(&mut self, path: PathBuf) {
        *self.pending_files.entry(path).or_insert(0) += 1;
    }

    pub fn add_directory(&mut self, path: PathBuf) {
        *self.pending_directories.entry(path).or_insert(0) += 1;
    }

    pub fn remove_file(&mut self, path: PathBuf) {
        *self.pending_files.entry(path).or_insert(0) -= 1;
    }

    pub fn remove_directory(&mut self, path: PathBuf) {
        *self.pending_directories.entry(path).or_insert(0) -= 1;
    }

    pub fn commit(&mut self) {
        let files = std::mem::take(&mut self.pending_files);
        let directories = std::mem::take(&mut self.pending_directories);

        for (file, count) in files {
            if count > 0 {
                self.active_watches
                    .add_file(file, count.try_into().unwrap());
            } else if count < 0 {
                self.active_watches
                    .remove(&file, (-count).try_into().unwrap());
            }
        }

        for (directory, count) in directories {
            if count > 0 {
                self.active_watches
                    .add_directory(directory, count.try_into().unwrap());
            } else if count < 0 {
                self.active_watches
                    .remove(&directory, (-count).try_into().unwrap());
            }
        }
    }
}
