use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn cli_starts_and_shows_session_from_postgres_eventlog() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    let bin = env!("CARGO_BIN_EXE_harness-cli");

    let migrate = Command::new(bin)
        .args(["migrate", "--database-url", &database_url])
        .output()?;
    assert!(migrate.status.success());

    let repo_path = std::env::current_dir()?.display().to_string();
    let start = Command::new(bin)
        .args([
            "session",
            "start",
            "--repo",
            &repo_path,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(start.status.success());

    let start_stdout = String::from_utf8(start.stdout)?;
    let session_id = value_for_key(&start_stdout, "session_id").expect("session_id output");
    let repo_path = value_for_key(&start_stdout, "repo_path").expect("repo_path output");

    let show = Command::new(bin)
        .args([
            "session",
            "show",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(show.status.success());

    let show_stdout = String::from_utf8(show.stdout)?;
    assert!(show_stdout.contains("status=started"));
    assert!(show_stdout.contains("event_count=1"));
    assert!(show_stdout.contains(&format!("repo_path={repo_path}")));

    let verify = Command::new(bin)
        .args([
            "session",
            "verify-command",
            session_id,
            "--database-url",
            &database_url,
            "--",
            "cargo",
            "--version",
        ])
        .output()?;
    assert!(verify.status.success());

    let verify_stdout = String::from_utf8(verify.stdout)?;
    assert!(verify_stdout.contains("policy_decision=allow"));
    assert!(verify_stdout.contains("tool_executed=true"));
    assert!(verify_stdout.contains("exit_code=0"));
    assert!(verify_stdout.contains("event_count=4"));

    Ok(())
}

#[test]
fn cli_compiles_session_context() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    let bin = env!("CARGO_BIN_EXE_harness-cli");

    let migrate = Command::new(bin)
        .args(["migrate", "--database-url", &database_url])
        .output()?;
    assert!(migrate.status.success());

    let repo_path = std::env::current_dir()?.display().to_string();
    let start = Command::new(bin)
        .args([
            "session",
            "start",
            "--repo",
            &repo_path,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(start.status.success());

    let start_stdout = String::from_utf8(start.stdout)?;
    let session_id = value_for_key(&start_stdout, "session_id").expect("session_id output");

    let compile = Command::new(bin)
        .args([
            "session",
            "compile-context",
            session_id,
            "--database-url",
            &database_url,
            "--max-bytes",
            "4096",
            "--focus",
            "agent",
        ])
        .output()?;
    assert!(compile.status.success());

    let compile_stdout = String::from_utf8(compile.stdout)?;
    assert!(compile_stdout.contains("context_sources="));
    assert!(compile_stdout.contains("context_used_bytes="));
    assert!(compile_stdout.contains("context_truncated=false"));
    assert!(compile_stdout.contains("event_count=2"));

    Ok(())
}

#[test]
fn cli_runs_fake_model_turn() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    let bin = env!("CARGO_BIN_EXE_harness-cli");
    let repo = fixture_repo()?;
    write_file(&repo, "AGENTS.md", "Use deterministic agent fixtures.")?;

    let migrate = Command::new(bin)
        .args(["migrate", "--database-url", &database_url])
        .output()?;
    assert!(migrate.status.success());

    let repo_path = repo.display().to_string();
    let start = Command::new(bin)
        .args([
            "session",
            "start",
            "--repo",
            &repo_path,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(start.status.success());

    let start_stdout = String::from_utf8(start.stdout)?;
    let session_id = value_for_key(&start_stdout, "session_id").expect("session_id output");

    let turn = Command::new(bin)
        .args([
            "session",
            "fake-turn",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the CLI fake patch",
            "--max-bytes",
            "4096",
            "--focus",
            "agent",
        ])
        .output()?;
    assert!(turn.status.success());

    let turn_stdout = String::from_utf8(turn.stdout)?;
    let written = fs::read_to_string(repo.join(".harness/fake-agent-turn.md"))?;
    assert!(turn_stdout.contains("patch_path=.harness/fake-agent-turn.md"));
    assert!(turn_stdout.contains("policy_decision=allow"));
    assert!(turn_stdout.contains("patch_applied=true"));
    assert!(turn_stdout.contains("event_count=7"));
    assert!(written.contains("write the CLI fake patch"));

    Ok(())
}

fn database_url() -> Option<String> {
    std::env::var("HARNESS_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("HARNESS_DATABASE_URL"))
        .ok()
}

fn value_for_key<'a>(output: &'a str, key: &str) -> Option<&'a str> {
    output
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
}

fn fixture_repo() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = std::env::temp_dir().join(format!(
        "coding-agent-harness-cli-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn write_file(
    root: &Path,
    relative_path: &str,
    content: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = root.join(relative_path);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(path)?;
    file.write_all(content.as_bytes())?;
    Ok(())
}
