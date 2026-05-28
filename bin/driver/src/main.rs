use std::ops::Deref;
use std::path::PathBuf;

use clap::{Arg, ArgAction, Command, arg, command};
use futures_lite::future;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

use driver_query_ssg::QueryContext;

mod fs;

fn main() {
    match real_main() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

fn time<T>(message: &'static str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let out = f();
    println!("{}: {:?}", message, start.elapsed());
    out
}

fn real_main() -> driver_util::Result<()> {
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
            .arg(arg!(<script> "The file to run"))
            .arg(Arg::new("remaining").last(true).action(ArgAction::Append)))
        .subcommand(Command::new("print-graph"))
        .get_matches();

    let dist = PathBuf::from(matches.get_one("dist").unwrap_or(&"./dist".to_string()));
    let cache = PathBuf::from(matches.get_one("cache").unwrap_or(&"./.driver".to_string()));
    let options = driver_engine::Options {
        cache_path: cache.join("cache.zst"),
        remotes_path: cache.join("remotes.zst"),
        objects_path: cache.join("objects"),
    };

    let root = time("restored database", || QueryContext::create_root(options));

    if let Some(run_matches) = matches.subcommand_matches("run") {
        let filename = run_matches
            .get_one::<String>("script")
            .expect("<script> must be provided.");
        let write_options = fs::WriteOptions {
            output_path: dist,
            no_delete_missing: run_matches.get_flag("no-delete-missing"),
        };
        let args = run_matches
            .get_many::<String>("remaining")
            .unwrap_or_default()
            .map(|s| s.deref());

        let output = time("ran query", || {
            future::block_on(fs::run(&root, filename.into(), args))
        })?;
        time("wrote output", || {
            future::block_on(output.write(&root, &write_options))
        })?;
    }

    if matches.subcommand_matches("print-graph").is_some() {
        println!("{}", root.db().display_dep_graph());
    }

    time("saved database", || root.destroy_root())?;

    Ok(())
}
