use clap::arg;
use clap::command;

fn main() -> std::io::Result<()> {
    let matches = command!()
        .arg(arg!(script: "The directory to hash"))
        .get_matches();

    let filename = match matches.get_one::<String>("script") {
        Some(f) => f,
        None => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "missing required argument",
            ));
        }
    };

    let ctx = query::QueryContext::default();
    let digest = query::walk(filename.into(), &ctx);
    println!("{digest:?}");

    std::io::stdin().read_line(&mut String::new())?;

    ctx.new_revision();
    let digest = query::walk(filename.into(), &ctx);
    println!("{digest:?}");

    Ok(())
}
