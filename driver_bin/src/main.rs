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

    if filename == "koto" {
        query::koto("let x = 1; let y = 1; || 1 + 2", &ctx);
        query::koto("|| 1 + 2", &ctx);
        return Ok(());
    }

    let digest = query::walk(filename.into(), &ctx);
    println!("{digest:?}");

    loop {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        if s.as_bytes().first() == Some(&b'q') {
            return Ok(());
        }

        ctx.new_revision();
        let digest = query::walk(filename.into(), &ctx);
        println!("{digest:?}");
    }
}
