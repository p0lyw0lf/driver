use std::sync::Arc;

use clap::arg;
use clap::command;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

fn main() -> query::Result<()> {
    let start = std::time::SystemTime::now();

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

    println!("parsed cli args: {:?}", start.elapsed()?);

    rt.block_on(async {
        let ctx = query::QueryContext::restore_or_default().await;

        println!("restored database: {:?}", start.elapsed()?);

        if let Some(filename) = matches.get_one::<String>("script") {
            let output = query::run(filename.into(), &ctx).await?;
            println!("ran query: {:?}", start.elapsed()?);
            output.write(&ctx).await?;
            println!("wrote output: {:?}", start.elapsed()?);
        }

        if matches.get_flag("print_graph") {
            println!("{}", ctx.display_dep_graph());
        }

        ctx.save().await?;

        println!("saved database: {:?}", start.elapsed()?);

        Ok(())
    })
}
