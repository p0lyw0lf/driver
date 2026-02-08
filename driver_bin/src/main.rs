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
    let run = || {
        if filename.ends_with("js") {
            if let Err(e) = query::run(filename.into(), &ctx) {
                eprintln!("{e}");
            }
        } else {
            let digest = query::walk(filename.into(), &ctx);
            println!("{digest:?}");
        }
    };
    run();

    loop {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        if s.as_bytes().first() == Some(&b'q') {
            return Ok(());
        }

        ctx.new_revision();
        println!("running again");
        run();
    }
}
