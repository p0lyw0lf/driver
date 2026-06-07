use std::path::PathBuf;

driver_engine::key!(
    #[input=|_| true]
    struct ListDirectory(pub PathBuf);
);
driver_engine::no_objects!(ListDirectory);

driver_engine::producer!(ListDirectory(self, ctx) -> driver_util::Result<Vec<PathBuf>> {
    // TODO: make this async? Unclear if worth it, investigate later
    let walk = ignore::WalkBuilder::new(&self.0)
        .max_depth(Some(1))
        .sort_by_file_name(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .build();
    let entries = walk
        .into_iter()
        .map(|e| e.map(|entry| entry.into_path()))
        .filter(|e| match e {
            // Pass thru all errors
            Err(_) => true,
            // Exclude the target directory from the returned list
            Ok(entry) => entry != &self.0,
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(entries)
});

impl std::fmt::Display for ListDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "list_directory(\"{}\")", self.0.display())
    }
}
