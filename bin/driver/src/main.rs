use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::ops::Deref;
use std::path::PathBuf;

use clap::{Arg, ArgAction, Command, arg, command, value_parser};
use futures_lite::future;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use driver_query_ssg::QueryContext;

mod fs;
mod http;

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
        .arg(arg!(--cache <dir> "Where to save the cache to. Default: ./.driver"))
        .subcommand(Command::new("run")
            .long_about("Runs a Javascript file, writing all files it outputs")
            .arg(arg!(--dist <dir> "Where to output the files to. Default: ./dist"))
            .arg(arg!(--"no-delete-missing" "Only adds new output files, never deletes old ones"))
            .arg(arg!(<script> "The file to run").value_parser(value_parser!(PathBuf)))
            .arg(Arg::new("remaining").last(true).action(ArgAction::Append)).long_about("These arguments are provided as an array of strings to the file being run."))
        .subcommand(Command::new("serve")
            .long_about("Runs an HTTP server, streaming files as responses directly")
            .arg(arg!(-h --host <host> "The host to listen on. Default: localhost.").value_parser(value_parser!(IpAddr)))
            .arg(arg!(-p --port <port> "The port to listen on. Default: chosen by OS.").value_parser(value_parser!(u16)))
            .arg(arg!(<script> "The file to run").value_parser(value_parser!(PathBuf)))
            .arg(Arg::new("remaining").last(true).action(ArgAction::Append)).long_about("\
                The first argument provided to the script is the filename it should generate a default export for; \
                these are appended afterwards."))
        .subcommand(Command::new("print-graph").arg(arg!(--"with-outputs" "In addition to printing each dependency key, also print each dependency output")))
        .get_matches();

    let cache = PathBuf::from(matches.get_one("cache").unwrap_or(&"./.driver".to_string()));
    let options = driver_engine::Options::with_base_dir(&cache);

    let root = time("restored database", || QueryContext::create_root(options));

    if let Some(run_matches) = matches.subcommand_matches("run") {
        let filename = run_matches
            .get_one::<PathBuf>("script")
            .expect("<script> must be provided.");
        let dist = PathBuf::from(run_matches.get_one("dist").unwrap_or(&"./dist".to_string()));
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

    if let Some(serve_matches) = matches.subcommand_matches("serve") {
        let filename = serve_matches
            .get_one::<PathBuf>("script")
            .expect("<script> must be provided.");

        let bind_addr = {
            let host = serve_matches
                .get_one::<IpAddr>("host")
                .unwrap_or(&IpAddr::V4(Ipv4Addr::LOCALHOST));
            let port = serve_matches.get_one::<u16>("port").unwrap_or(&0);
            SocketAddr::new(*host, *port)
        };
        let args = serve_matches
            .get_many::<String>("remaining")
            .unwrap_or_default()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();

        // TODO: file watcher that invalidates specific input queries for root
        http::serve(bind_addr, &root, filename.clone(), args)?;
    }

    if let Some(print_matches) = matches.subcommand_matches("print-graph") {
        if print_matches.get_flag("with-outputs") {
            println!("{}", root.db().display_dep_graph_with_outputs());
        } else {
            println!("{}", root.db().display_dep_graph());
        }
    }

    time("saved database", || root.destroy_root())?;

    Ok(())
}
