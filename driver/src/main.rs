use std::sync::Arc;

use clap::{arg, command};
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

    let matches = command!()
        .arg(arg!(--print_graph "Prints the saved dependency graph"))
        .arg(arg!([script] "The file to run"))
        .get_matches();

    println!("parsed cli args: {:?}", start.elapsed()?);

    let rt = Arc::new(query::Executor::start(query::Options::default()));
    println!("restored database: {:?}", start.elapsed()?);

    if let Some(filename) = matches.get_one::<String>("script") {
        let output = future::block_on(query::run(rt.clone(), filename.into()))?;
        println!("ran query: {:?}", start.elapsed()?);
        future::block_on(output.write(&rt))?;
        println!("wrote output: {:?}", start.elapsed()?);
    }

    if matches.get_flag("print_graph") {
        println!("{}", rt.display_dep_graph());
    }

    let rt = Arc::into_inner(rt).expect("was still running");
    rt.stop()?;

    println!("saved database: {:?}", start.elapsed()?);

    Ok(())
}
