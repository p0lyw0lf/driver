use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

mod db;
mod files;
// TODO: come up with a better name for this module
mod keys;

use keys::AnyOutput;
use keys::Hash;
use keys::ToHash;
use sha2::Digest;

// TODO: use a macro to generate these cases and From impls
#[derive(Hash, PartialEq, Eq, Clone)]
enum QueryKey {
    ReadFile(files::ReadFile),
    ListDirectory(files::ListDirectory),

    // This will depend on ReadFile, and if that ReadFile is red, every other dependency will need
    // to be removed & re-generated. This probably requires a bit tighter integration with the
    // query system...
    // output: the exported object, as well as all the "side-effects" (writes to files)
    ExecuteFile(PathBuf),

    // This will depend on the arguments to the function as well as the function object itself.
    // Honestly I have no idea how to serialize/de-serialize these properly, leaving that for the
    // future though.
    // When ExecuteFile is called, it will have many of these as dependencies. These will get
    // re-run if the arguments or the function objects changes. Meaning if this doesn't change, the
    // return value and "side-effects" it produces will also stay the same.
    ExecuteFunction { args: Vec<()>, function: () },
}

impl From<files::ReadFile> for QueryKey {
    fn from(value: files::ReadFile) -> Self {
        Self::ReadFile(value)
    }
}

impl From<files::ListDirectory> for QueryKey {
    fn from(value: files::ListDirectory) -> Self {
        Self::ListDirectory(value)
    }
}

trait Producer {
    type Output: Sized + ToHash;
    fn produce(&self, ctx: &QueryContext) -> anyhow::Result<Self::Output>;
    fn query(self, ctx: &QueryContext) -> anyhow::Result<Self::Output>
    where
        Self: Sized,
        QueryKey: From<Self>,
    {
        let b: Box<dyn std::any::Any> = ctx.query(QueryKey::from(self))?.0;
        Ok(*b.downcast::<Self::Output>().expect("invalid inner"))
    }
}

impl Producer for QueryKey {
    type Output = AnyOutput;
    fn produce(&self, ctx: &QueryContext) -> anyhow::Result<Self::Output> {
        Ok(match self {
            QueryKey::ReadFile(read_file) => AnyOutput::new(read_file.produce(ctx)?),
            QueryKey::ListDirectory(list_directory) => AnyOutput::new(list_directory.produce(ctx)?),
            QueryKey::ExecuteFile(path_buf) => todo!(),
            QueryKey::ExecuteFunction { args, function } => todo!(),
        })
    }
}

pub struct QueryContext {
    parent: Option<QueryKey>,
    db: Arc<db::Database>,
    dep_graph: Arc<db::DepGraph>,
}

impl QueryContext {
    fn query(&self, key: QueryKey) -> anyhow::Result<AnyOutput> {
        let revision = self.db.revision.load(Ordering::SeqCst);
        let update_value = |key: QueryKey| -> anyhow::Result<()> {
            if let Some(parent) = &self.parent {
                self.dep_graph.add_dependency(parent.clone(), key.clone());
            }

            let value = key.produce(&QueryContext {
                parent: Some(key.clone()),
                db: self.db.clone(),
                dep_graph: self.dep_graph.clone(),
            })?;
            let hash = value.to_hash();

            // TODO: I would like to be able to clone here, but type-erasing doesn't work because
            // you can't make a Box<dyn Clone> because of dyn-compatibility. This sucks I wonder
            // what the way to do this is.
            let old = self.db.cache.insert(key.clone(), (value, hash));
            if old.is_some_and(|old| old.1 == hash) {
                self.db.colors.mark_green(&key, revision);
            } else {
                self.db.colors.mark_red(&key, revision);
            }

            Ok(())
        };

        let Some((_, rev)) = self.db.colors.get(&key) else {
            update_value(key)?;
            return todo!();
        };
        if rev < revision {
            update_value(key)?;
            return todo!();
        }

        // TODO: finish this function implementation copying from https://thunderseethe.dev/posts/lsp-base/
        todo!()
    }
}

fn walk_impl(ctx: &QueryContext, dir: PathBuf) -> anyhow::Result<Hash> {
    let mut hasher = sha2::Sha256::new();
    for entry in files::ListDirectory(dir).query(ctx)? {
        let digest = if entry.is_dir() {
            walk_impl(ctx, entry)?
        } else {
            files::ReadFile(entry).query(ctx)?.to_hash()
        };
        hasher.update(digest);
    }
    Ok(hasher.finalize())
}

pub fn walk(dir: PathBuf) -> anyhow::Result<Hash> {
    walk_impl(
        &QueryContext {
            parent: None,
            db: Default::default(),
            dep_graph: Default::default(),
        },
        dir,
    )
}
