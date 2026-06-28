use std::ops::Deref;
use std::path::PathBuf;

use clap::{Arg, ArgAction, Command, arg, command, value_parser};
use futures_lite::future;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use driver_query_wasm::QueryContext;

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
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| ["warn"].join(",").into()))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .without_time(),
        )
        .init();

    let _ = include_str!("../Cargo.toml");
    let matches = command!()
        .arg(arg!(--cache <dir> "Where to save the cache.").value_parser(value_parser!(PathBuf)).default_value("./.driver"))
        .subcommand(Command::new("run")
            .long_about("Runs a Javascript file, writing all files it outputs")
            .arg(arg!(--dist <dir> "Where to output the files.").value_parser(value_parser!(PathBuf)).default_value("./dist"))
            .arg(arg!(--"no-delete-missing" "Only adds new output files, never deletes old ones"))
            .arg(arg!(<script> "The file to run").value_parser(value_parser!(PathBuf)))
            .arg(Arg::new("remaining").last(true).action(ArgAction::Append)).long_about("These arguments are provided as an array of strings to the file being run."))
        .subcommand(Command::new("print-graph").arg(arg!(--"with-outputs" "In addition to printing each dependency key, also print each dependency output")))
        .subcommand(Command::new("clean").about("Allows for cleaning the database and object store.")
            .arg(arg!(--key <prefix> "Removes all keys starting with the given prefix from the database").action(ArgAction::Append))
            .arg(arg!(--db "Cleans the entire database"))
            .arg(arg!(--remotes "Cleans the remote cache"))
            .arg(arg!(--dist [dir] "Cleans the output directory.").value_parser(value_parser!(PathBuf)).default_value("./dist"))
            .arg(arg!(--gc "Keeps only objects in the object store that are referenced in either the db or remote cache"))
        )
        .get_matches();

    let cache = matches
        .get_one::<PathBuf>("cache")
        .expect("--cache must be provided");
    let options = driver_engine::Options::with_base_dir(cache);

    let root = time("restored database", || QueryContext::create_root(options));

    if let Some(run_matches) = matches.subcommand_matches("run") {
        let filename = run_matches
            .get_one::<PathBuf>("script")
            .expect("<script> must be provided.");
        let dist = run_matches
            .get_one::<PathBuf>("dist")
            .expect("--dist must be provided");
        let write_options = fs::WriteOptions {
            output_path: dist.clone(),
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

    if let Some(print_matches) = matches.subcommand_matches("print-graph") {
        if print_matches.get_flag("with-outputs") {
            println!("{}", root.db().display_dep_graph_with_outputs());
        } else {
            println!("{}", root.db().display_dep_graph());
        }
    }

    if let Some(forget_matches) = matches.subcommand_matches("clean") {
        if forget_matches.get_flag("db") {
            // Delete entire database
            root.db().clear();
        } else if let Some(prefixes) = forget_matches.get_many::<String>("key") {
            let prefixes: Vec<&String> = prefixes.collect();
            root.db().remove_keys_matching_prefixes(&prefixes);
        }

        if forget_matches.get_flag("remotes") {
            // Delete remote cache
            root.db().clear_remote();
        }

        if let Some(dist) = forget_matches.get_one::<PathBuf>("dist") {
            // Delete all keys with no parents (keys that we ran at the top level & produced output
            // from) from the database.
            root.db().remove_root_keys();
            // Then, actually delete the output directory.
            std::fs::remove_dir_all(dist)?;
        }

        if forget_matches.get_flag("gc") {
            root.db().garbage_collect(root.options())?;
        }
    }

    time("saved database", || root.destroy_root())?;

    Ok(())
}
