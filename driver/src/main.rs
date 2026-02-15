use clap::arg;
use clap::command;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{EnvFilter, fmt};

fn main() -> query::Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_span_events(FmtSpan::ACTIVE)
        .without_time() // TODO: remove once debugging complete
        .init();

    let matches = command!()
        .arg(arg!(--print_graph "Prints the saved dependency graph"))
        .arg(arg!([script] "The file to run"))
        .get_matches();

    let ctx = query::QueryContext::restore().unwrap_or_else(|e| {
        eprintln!("error restoring context: {e}");
        query::QueryContext::default()
    });

    if let Some(filename) = matches.get_one::<String>("script")
        && let Err(e) = query::run(filename.into(), &ctx)
    {
        eprintln!("{e}");
    }

    if matches.get_flag("print_graph") {
        println!("{}", ctx.display_dep_graph());
    }

    ctx.save()?;
    Ok(())
}
