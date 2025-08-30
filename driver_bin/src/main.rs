use driver::drv::Derivation;

fn main() -> std::io::Result<()> {
    let drv1 = Derivation {
        builder: vec!["echo".to_string(), "one".to_string()],
        output_path: "1".to_string(),
    };
    let drv2 = Derivation {
        builder: vec!["echo".to_string(), "two".to_string()],
        output_path: "2".to_string(),
    };
    let drv3 = Derivation {
        builder: vec![
            "cat".to_string(),
            drv1.output_path().to_str().unwrap().to_string(),
            drv2.output_path().to_str().unwrap().to_string(),
        ],
        output_path: "3".to_string(),
    };

    drv1.run()?;
    drv2.run()?;
    drv3.run()?;

    Ok(())
}
