use clap::arg;
use clap::command;
use driver::engine;

fn main() -> std::io::Result<()> {
    let matches = command!()
        .arg(arg!(script: "The build plan script to run"))
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

    let derivations = engine::run_script(filename)
        .map_err(|err| std::io::Error::other(format!("error running script: {}", err)))?;
    engine::run_derivations(derivations)?;

    Ok(())
}
