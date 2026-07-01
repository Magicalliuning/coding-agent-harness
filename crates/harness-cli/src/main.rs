use std::env;

fn main() {
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        Some("--version") | Some("-V") => {
            println!(
                "{} {}",
                harness_core::PRODUCT_NAME,
                env!("CARGO_PKG_VERSION")
            );
        }
        Some("doctor") | None => {
            println!("{}", harness_runtime::doctor_report());
        }
        Some(other) => {
            eprintln!("unknown command: {other}");
            eprintln!("usage: harness-cli [--version|doctor]");
            std::process::exit(2);
        }
    }
}
