use std::sync::Arc;

use clap::arg;
use clap::command;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

fn main() -> query::Result<()> {
    let console_layer = console_subscriber::Builder::default()
        .with_default_env()
        .spawn();

    let fmt_layer = fmt::layer()
        .with_ansi(false)
        .without_time()
        .with_filter(EnvFilter::from_default_env());

    tracing_subscriber::registry()
        .with(console_layer)
        .with(fmt_layer)
        .init();

    let matches = command!()
        .arg(arg!(--print_graph "Prints the saved dependency graph"))
        .arg(arg!([script] "The file to run"))
        .get_matches();

    // Don't need multithreading since things will be mostly limited by I/O & javascript single
    // thread anyways. Just need concurrency.
    let rt = Arc::new(tokio::runtime::Builder::new_current_thread().build()?);

    rt.block_on(async {
        let ctx = query::QueryContext::restore_or_default(rt.clone()).await;

        if let Some(filename) = matches.get_one::<String>("script")
            && let Err(e) = query::run(filename.into(), &ctx).await
        {
            eprintln!("{e}");
        }

        if matches.get_flag("print_graph") {
            println!("{}", ctx.display_dep_graph());
        }

        ctx.save().await?;
        Ok(())
    })
}
