use std::path::PathBuf;

use crate::QueryContext;

#[derive(Hash, PartialEq, Eq, Clone)]
pub struct ReadFile(pub PathBuf);

impl crate::Producer for ReadFile {
    type Output = String;
    fn produce(&self, _ctx: &QueryContext) -> anyhow::Result<Self::Output> {
        Ok(String::from_utf8(std::fs::read(&self.0)?)?)
    }
}

#[derive(Hash, PartialEq, Eq, Clone)]
pub struct ListDirectory(pub PathBuf);

impl crate::Producer for ListDirectory {
    type Output = Vec<PathBuf>;
    fn produce(&self, _ctx: &QueryContext) -> anyhow::Result<Self::Output> {
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
    }
}
