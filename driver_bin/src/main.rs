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

    let digest = dirhash::walk(filename.into());
    println!("{digest:?}");

    Ok(())
}
