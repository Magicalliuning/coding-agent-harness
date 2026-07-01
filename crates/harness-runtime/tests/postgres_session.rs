use harness_runtime::{Runtime, SessionStatus, StartSessionRequest};
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

fn database_url() -> Option<String> {
    std::env::var("HARNESS_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("HARNESS_DATABASE_URL"))
        .ok()
}
