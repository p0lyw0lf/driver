use nix_base32::from_nix_base32;

use driver::drv::BuildDerivation;
use driver::drv::Derivation;
use driver::drv::FileDerivation;

fn main() -> std::io::Result<()> {
    let drv1 = FileDerivation {
        input_path: "1".to_string(),
        digest: from_nix_base32("1pjg92lyms0y5vqiwqh8hqlyjgd3bd96608l5j7xgfs11ay1nanm").unwrap(),
    };
    let drv2 = FileDerivation {
        input_path: "2".to_string(),
        digest: from_nix_base32("0nha603ll601ikc24jd0kskbr32sxl909zgrazar9zw39ba8xp97").unwrap(),
    };

    let drv3 = BuildDerivation {
        builder: vec![
            "python".to_string(),
            "-c".to_string(),
            format!(
                r#"
with open("{}/1", "r") as f1, \
    open("{}/2", "r") as f2, \
    open("$out/3", "w") as f3:
    f3.write(f1.read())
    f3.write(f2.read())
            "#,
                drv1.output_path().to_str().unwrap(),
                drv2.output_path().to_str().unwrap(),
            ),
        ],
    };

    println!("running drv1...");
    drv1.run()?;
    println!("running drv2...");
    drv2.run()?;
    println!("running drv3...");
    drv3.run()?;

    Ok(())
}
