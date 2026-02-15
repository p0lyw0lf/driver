use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::db::object::Object;
use crate::query::context::Producer;
use crate::query::context::QueryContext;
use crate::query_key;

query_key!(ReadFile(pub PathBuf););

impl Producer for ReadFile {
    type Output = crate::Result<Object>;
    #[tracing::instrument(level = "debug", skip(ctx))]
    fn produce(&self, ctx: &QueryContext) -> Self::Output {
        // println!("reading: {}", self.0.display());
        let content = std::fs::read(&self.0)?;
        let object = ctx.db.objects.store(content);
        Ok(object)
    }
}

query_key!(ListDirectory(pub PathBuf););

impl Producer for ListDirectory {
    type Output = crate::Result<Vec<PathBuf>>;
    #[tracing::instrument(level = "debug", skip(_ctx))]
    fn produce(&self, _ctx: &QueryContext) -> Self::Output {
        // println!("walking: {}", self.0.display());
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
