use std::path::PathBuf;
use std::sync::Arc;

use clap::{Command, arg, command};
use futures_lite::future;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

fn main() -> query::Result<()> {
    let start = std::time::SystemTime::now();

    let fmt_layer = fmt::layer()
        .with_ansi(false)
        .without_time()
        .with_filter(EnvFilter::from_default_env());

    tracing_subscriber::registry().with(fmt_layer).init();

    let _ = include_str!("../Cargo.toml");
    let matches = command!()
        .arg(arg!(--dist <dir> "Where to output the files to. Default: ./dist"))
        .arg(arg!(--cache <dir> "Where to save the cache to. Default: ./.driver"))
        .subcommand(Command::new("run")
            .arg(arg!(--"no-delete-missing" "Only adds new output files, never deletes old ones"))
            .arg(arg!(<script> "The file to run")))
        .subcommand(Command::new("print-graph"))
        .get_matches();

    let dist = PathBuf::from(matches.get_one("dist").unwrap_or(&"./dist".to_string()));
    let cache = PathBuf::from(matches.get_one("cache").unwrap_or(&"./.driver".to_string()));
    let options = query::Options {
        output_path: dist,
        cache_path: cache.join("cache.zst"),
        remotes_path: cache.join("remotes.zst"),
        objects_path: cache.join("objects"),
    };

    println!(
        "parsed cli args: {:?} {:?}",
        start.elapsed()?,
        std::env::args()
    );

    let rt = Arc::new(query::Executor::start(options));
    println!("restored database: {:?}", start.elapsed()?);

    if let Some(run_matches) = matches.subcommand_matches("run") {
        let filename = run_matches
            .get_one::<String>("script")
            .expect("<script> must be provided.");
        let write_options = query::WriteOptions {
            no_delete_missing: run_matches.get_flag("no-delete-missing"),
        };

        let output = future::block_on(query::run(rt.clone(), filename.into()))?;
        println!("ran query: {:?}", start.elapsed()?);
        future::block_on(output.write(&rt, &write_options))?;
        println!("wrote output: {:?}", start.elapsed()?);
    }

    if matches.subcommand_matches("print-graph").is_some() {
        println!("{}", rt.display_dep_graph());
    }

    let rt = Arc::into_inner(rt).expect("was still running");
    rt.stop()?;

    println!("saved database: {:?}", start.elapsed()?);

    Ok(())
}
