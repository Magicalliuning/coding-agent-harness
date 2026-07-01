use harness_policy::PolicyDecision;
use harness_runtime::{
    ContextBudget, Runtime, SessionContextCompileRequest, SessionStatus, StartSessionRequest,
    VerificationCommandRequest,
};
use uuid::Uuid;

#[test]
fn started_session_can_be_replayed_from_postgres_eventlog() -> Result<(), Box<dyn std::error::Error>>
{
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo_path = format!("fixture-repo-{}", Uuid::new_v4());
    let mut runtime = Runtime::connect_postgres(&database_url)?;

    let started = runtime.start_session(StartSessionRequest::new(repo_path.clone()))?;
    let replayed = runtime.show_session(started.session_id)?;

    assert_eq!(started.session_id, replayed.session_id);
    assert_eq!(replayed.repo_path, repo_path);
    assert_eq!(replayed.status, SessionStatus::Started);
    assert_eq!(replayed.event_count, 1);

    Ok(())
}

#[test]
fn verification_command_records_allowed_observation() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo_path = workspace_root()?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path))?;

    let result = runtime.run_verification_command(
        started.session_id,
        VerificationCommandRequest::new("cargo", vec!["--version".to_owned()]),
    )?;
    let replayed = runtime.show_session(started.session_id)?;

    assert_eq!(result.decision, PolicyDecision::Allow);
    assert!(result.observation.is_some());
    assert_eq!(result.event_count, 4);
    assert_eq!(replayed.event_count, 4);

    Ok(())
}

#[test]
fn session_context_compilation_is_recorded() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo_path = workspace_root()?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path))?;

    let result = runtime.compile_session_context(
        started.session_id,
        SessionContextCompileRequest {
            budget: ContextBudget {
                max_bytes: 4096,
                max_files: 8,
                max_skill_files: 8,
            },
            focus_terms: vec!["agent".to_owned()],
        },
    )?;

    assert_eq!(result.event_count, 2);
    assert!(
        result
            .bundle
            .sources
            .iter()
            .any(|source| source.path == "AGENTS.md")
    );

    Ok(())
}

#[test]
fn denied_verification_command_is_not_executed() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo_path = std::env::current_dir()?.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path))?;

    let result = runtime.run_verification_command(
        started.session_id,
        VerificationCommandRequest::new("git", vec!["reset".to_owned(), "--hard".to_owned()]),
    )?;

    assert_eq!(result.decision, PolicyDecision::Deny);
    assert_eq!(result.observation, None);
    assert_eq!(result.event_count, 3);

    Ok(())
}

#[test]
fn approval_required_verification_command_is_not_executed() -> Result<(), Box<dyn std::error::Error>>
{
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo_path = std::env::current_dir()?.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path))?;

    let result = runtime.run_verification_command(
        started.session_id,
        VerificationCommandRequest::new("rustc", vec!["--version".to_owned()]),
    )?;

    assert_eq!(result.decision, PolicyDecision::Ask);
    assert_eq!(result.observation, None);
    assert_eq!(result.event_count, 3);

    Ok(())
}

fn database_url() -> Option<String> {
    std::env::var("HARNESS_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("HARNESS_DATABASE_URL"))
        .ok()
}

fn workspace_root() -> Result<String, Box<dyn std::error::Error>> {
    Ok(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?
        .display()
        .to_string())
}
