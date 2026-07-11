//! A couple of the modules internally depend on being able to access _all_ keys (specifically, the
//! boa and tera modules), so we need to make a concrete type here instead of higher up.
//!
//! I would like to not have to do this, but the alternatives are Much Worse + I don't see anyone
//! besides myself actually using this, so it'll do lol.

use driver_query_fs::{ListDirectory, ReadFile};
use driver_query_hyper::GetUrl;

use crate::boa::RunJs;
use crate::comrak::MarkdownToHtml;
use crate::minify_html::MinifyHtml;
use crate::tera::RunTera;
use crate::zune::{ConvertImage, ParseImage};

driver_engine::query!(
QueryKey {
    ReadFile,
    ListDirectory,
    GetUrl,
    RunJs,
    MarkdownToHtml,
    MinifyHtml,
    RunTera,
    ConvertImage,
    ParseImage,
} with QueryOutput);

pub type QueryContext = driver_engine::Context<QueryKey>;
pub type HashKey = driver_db::Hashed<QueryKey>;
pub type WriteOutput = driver_util::WriteOutput<HashKey>;
pub type WriteOutputBuilder = driver_util::WriteOutputBuilder<HashKey>;
