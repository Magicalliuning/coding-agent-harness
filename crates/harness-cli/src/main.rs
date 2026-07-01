use std::env;
use std::path::PathBuf;

use harness_core::{HarnessError, HarnessResult};
use harness_runtime::{Runtime, SessionProjection, StartSessionRequest};
use uuid::Uuid;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> HarnessResult<()> {
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        Some("--version") | Some("-V") => {
            println!(
                "{} {}",
                harness_core::PRODUCT_NAME,
                env!("CARGO_PKG_VERSION")
            );
            Ok(())
        }
        Some("doctor") | None => {
            println!("{}", harness_runtime::doctor_report());
            Ok(())
        }
        Some("migrate") => {
            let database_url = database_url_from_args(args.collect())?;
            harness_runtime::apply_database_migrations(&database_url)?;
            println!("migrations=applied");
            Ok(())
        }
        Some("session") => run_session_command(args.collect()),
        Some(other) => Err(HarnessError::new(format!(
            "unknown command: {other}; usage: harness-cli [--version|doctor|migrate|session]"
        ))),
    }
}

fn run_session_command(args: Vec<String>) -> HarnessResult<()> {
    let Some(command) = args.first() else {
        return Err(HarnessError::new("session command is required"));
    };

    match command.as_str() {
        "start" => {
            let repo_path = required_arg_value(&args, "--repo")?;
            let repo_path = canonical_repo_path(repo_path)?;
            let database_url = database_url_from_args(args)?;
            let mut runtime = Runtime::connect_postgres(&database_url)?;
            let projection = runtime.start_session(StartSessionRequest::new(repo_path))?;

            print_projection(&projection);
            Ok(())
        }
        "show" => {
            let session_id = args
                .get(1)
                .ok_or_else(|| HarnessError::new("session id is required"))?;
            let session_id = Uuid::parse_str(session_id)
                .map_err(|error| HarnessError::new(error.to_string()))?;
            let database_url = database_url_from_args(args)?;
            let runtime = Runtime::connect_postgres(&database_url)?;
            let projection = runtime.show_session(session_id)?;

            print_projection(&projection);
            Ok(())
        }
        other => Err(HarnessError::new(format!(
            "unknown session command: {other}"
        ))),
    }
}

fn database_url_from_args(args: Vec<String>) -> HarnessResult<String> {
    optional_arg_value(&args, "--database-url")
        .map(ToOwned::to_owned)
        .or_else(|| env::var(harness_db::DATABASE_URL_ENV).ok())
        .ok_or_else(|| HarnessError::new("HARNESS_DATABASE_URL is not set"))
}

fn required_arg_value<'a>(args: &'a [String], name: &str) -> HarnessResult<&'a str> {
    optional_arg_value(args, name).ok_or_else(|| HarnessError::new(format!("{name} is required")))
}

fn optional_arg_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find_map(|window| (window[0] == name).then_some(window[1].as_str()))
}

fn canonical_repo_path(repo_path: &str) -> HarnessResult<String> {
    let path = PathBuf::from(repo_path);
    let canonical = path
        .canonicalize()
        .map_err(|error| HarnessError::new(error.to_string()))?;

    Ok(canonical.display().to_string())
}

fn print_projection(projection: &SessionProjection) {
    println!("session_id={}", projection.session_id);
    println!("status={}", projection.status.as_str());
    println!("repo_path={}", projection.repo_path);
    println!("event_count={}", projection.event_count);
}
