use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use harness_events::EventType;
use harness_policy::PolicyDecision;
use harness_runtime::{
    ContextBudget, FakeModelTurnRequest, PENDING_COMMIT_APPROVAL_STATE, RECOVERY_FAILED_STATE,
    RecoveryStopReason, Runtime, SelfRecoveryLoopRequest, SessionContextCompileRequest,
    SessionStatus, SmallCodingTaskRequest, StartSessionRequest, VerificationCommandRequest,
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
fn fake_model_turn_records_patch_observation() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = fixture_repo()?;
    write_file(&repo, "AGENTS.md", "Use deterministic agent fixtures.")?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;

    let result = runtime.run_fake_model_turn(
        started.session_id,
        FakeModelTurnRequest {
            task: "write the first fake patch".to_owned(),
            context: SessionContextCompileRequest {
                budget: ContextBudget {
                    max_bytes: 4096,
                    max_files: 8,
                    max_skill_files: 8,
                },
                focus_terms: vec!["agent".to_owned()],
            },
            max_output_tokens: 256,
        },
    )?;
    let written = fs::read_to_string(repo.join(".harness/fake-agent-turn.md"))?;
    let events = runtime.events_for_session(started.session_id)?;
    let event_types = events
        .iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();

    assert_eq!(result.decision, PolicyDecision::Allow);
    assert!(result.observation.is_some());
    assert!(written.contains("write the first fake patch"));
    assert_eq!(result.event_count, 7);
    assert_eq!(
        event_types,
        vec![
            EventType::SessionStarted,
            EventType::ContextCompiled,
            EventType::ModelRequestRecorded,
            EventType::ModelDecisionRecorded,
            EventType::ToolCallIntended,
            EventType::PolicyDecided,
            EventType::ToolObservationRecorded,
        ]
    );
    assert_eq!(
        events[3].payload["patch"]["path"],
        ".harness/fake-agent-turn.md"
    );
    assert_eq!(events[4].payload["tool_name"], "apply_file_patch");
    assert_eq!(events[6].payload["applied"], true);

    Ok(())
}

#[test]
fn small_coding_task_stops_at_pending_commit_approval() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = fixture_repo()?;
    write_file(&repo, "AGENTS.md", "Use deterministic agent fixtures.")?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;

    let result = runtime.run_small_coding_task(
        started.session_id,
        SmallCodingTaskRequest {
            task: "write the verified fake patch".to_owned(),
            context: SessionContextCompileRequest {
                budget: ContextBudget {
                    max_bytes: 4096,
                    max_files: 8,
                    max_skill_files: 8,
                },
                focus_terms: vec!["agent".to_owned()],
            },
            max_output_tokens: 256,
            verification: VerificationCommandRequest::new("cargo", vec!["--version".to_owned()]),
        },
    )?;
    let written = fs::read_to_string(repo.join(".harness/fake-agent-turn.md"))?;
    let events = runtime.events_for_session(started.session_id)?;
    let event_types = events
        .iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();

    assert_eq!(result.final_state, PENDING_COMMIT_APPROVAL_STATE);
    assert!(written.contains("write the verified fake patch"));
    assert_eq!(result.diff.files_changed, 1);
    assert!(result.diff.insertions > 0);
    assert_eq!(result.diff.deletions, 0);
    assert_eq!(result.diff.paths, vec![".harness/fake-agent-turn.md"]);
    assert_eq!(result.verification.decision, PolicyDecision::Allow);
    assert_eq!(
        result
            .verification
            .observation
            .as_ref()
            .and_then(|item| item.exit_code),
        Some(0)
    );
    assert_eq!(
        result.token_ledger.total_tokens,
        result.token_ledger.prompt_tokens + result.token_ledger.completion_tokens
    );
    assert_eq!(result.event_replay.total_events, 12);
    assert_eq!(
        result.event_replay.last_event_type.as_deref(),
        Some("commit.approval_pending")
    );
    assert_eq!(result.event_count, 12);
    assert_eq!(
        event_types,
        vec![
            EventType::SessionStarted,
            EventType::ContextCompiled,
            EventType::ModelRequestRecorded,
            EventType::ModelDecisionRecorded,
            EventType::ToolCallIntended,
            EventType::PolicyDecided,
            EventType::ToolObservationRecorded,
            EventType::ToolCallIntended,
            EventType::PolicyDecided,
            EventType::ToolObservationRecorded,
            EventType::DiffRecorded,
            EventType::CommitApprovalPending,
        ]
    );
    assert_eq!(events[10].payload["files_changed"], 1);
    assert_eq!(events[11].payload["state"], PENDING_COMMIT_APPROVAL_STATE);

    Ok(())
}

#[test]
fn self_recovery_fixture_records_initial_failure() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = fixture_repo()?;
    write_recovery_fixture_project(&repo)?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;

    let result = runtime.run_self_recovery_fixture_task(
        started.session_id,
        SelfRecoveryLoopRequest {
            task: "write the failing recovery fixture patch".to_owned(),
            context: SessionContextCompileRequest {
                budget: ContextBudget {
                    max_bytes: 4096,
                    max_files: 8,
                    max_skill_files: 8,
                },
                focus_terms: vec!["agent".to_owned()],
            },
            max_output_tokens: 256,
            verification: recovery_fixture_verification(),
            max_recovery_rounds: 0,
            max_repair_bytes: 4096,
        },
    )?;
    let events = runtime.events_for_session(started.session_id)?;

    assert_eq!(
        result.report.stop_reason,
        RecoveryStopReason::MaxRoundsReached
    );
    assert_eq!(result.final_state, RECOVERY_FAILED_STATE);
    assert_eq!(result.report.retry_count, 0);
    assert_eq!(result.report.repair_attempts, 0);
    assert_eq!(
        result.report.failure_classification.as_deref(),
        Some("fixture_missing_recovery_marker")
    );
    assert_ne!(
        result
            .initial_verification
            .observation
            .as_ref()
            .and_then(|item| item.exit_code),
        Some(0)
    );
    assert!(events.iter().any(|event| {
        event.event_type == EventType::RecoveryFailureClassified
            && event.payload["classification"] == "fixture_missing_recovery_marker"
    }));
    assert!(events.iter().any(|event| {
        event.event_type == EventType::RecoveryStopped
            && event.payload["stop_reason"] == "max_rounds_reached"
    }));

    Ok(())
}

#[test]
fn self_recovery_fixture_repairs_and_passes() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = fixture_repo()?;
    write_recovery_fixture_project(&repo)?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;

    let result = runtime.run_self_recovery_fixture_task(
        started.session_id,
        SelfRecoveryLoopRequest {
            task: "write the recoverable fixture patch".to_owned(),
            context: SessionContextCompileRequest {
                budget: ContextBudget {
                    max_bytes: 4096,
                    max_files: 8,
                    max_skill_files: 8,
                },
                focus_terms: vec!["agent".to_owned()],
            },
            max_output_tokens: 256,
            verification: recovery_fixture_verification(),
            max_recovery_rounds: 2,
            max_repair_bytes: 4096,
        },
    )?;
    let written = fs::read_to_string(repo.join(".harness/fake-agent-turn.md"))?;
    let events = runtime.events_for_session(started.session_id)?;

    assert_eq!(result.report.stop_reason, RecoveryStopReason::Recovered);
    assert_eq!(result.final_state, PENDING_COMMIT_APPROVAL_STATE);
    assert_eq!(result.report.retry_count, 1);
    assert_eq!(result.report.repair_attempts, 1);
    assert_eq!(result.report.max_recovery_rounds, 2);
    assert!(written.contains("recovered=true"));
    assert_eq!(
        result
            .final_verification
            .observation
            .as_ref()
            .and_then(|item| item.exit_code),
        Some(0)
    );
    assert!(result.diff.insertions > 0);
    assert_eq!(result.diff.paths, vec![".harness/fake-agent-turn.md"]);
    assert_eq!(
        result.event_replay.last_event_type.as_deref(),
        Some("recovery.stopped")
    );
    assert!(events.iter().any(|event| {
        event.event_type == EventType::RecoveryPlanRecorded
            && event.payload["round"] == 1
            && event.payload["plan"]
                .as_str()
                .is_some_and(|item| item.contains("recovered=true"))
    }));
    assert!(events.iter().any(|event| {
        event.event_type == EventType::RecoveryRepairAttempted
            && event.payload["round"] == 1
            && event.payload["applied"] == true
    }));
    assert!(events.iter().any(|event| {
        event.event_type == EventType::CommitApprovalPending
            && event.payload["state"] == PENDING_COMMIT_APPROVAL_STATE
    }));

    Ok(())
}

#[test]
fn self_recovery_fixture_stops_when_repair_budget_exceeded()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = fixture_repo()?;
    write_recovery_fixture_project(&repo)?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;

    let result = runtime.run_self_recovery_fixture_task(
        started.session_id,
        SelfRecoveryLoopRequest {
            task: "write the budget-limited recovery fixture patch".to_owned(),
            context: SessionContextCompileRequest {
                budget: ContextBudget {
                    max_bytes: 4096,
                    max_files: 8,
                    max_skill_files: 8,
                },
                focus_terms: vec!["agent".to_owned()],
            },
            max_output_tokens: 256,
            verification: recovery_fixture_verification(),
            max_recovery_rounds: 2,
            max_repair_bytes: 1,
        },
    )?;
    let written = fs::read_to_string(repo.join(".harness/fake-agent-turn.md"))?;
    let events = runtime.events_for_session(started.session_id)?;

    assert_eq!(
        result.report.stop_reason,
        RecoveryStopReason::RepairBudgetExceeded
    );
    assert_eq!(result.final_state, RECOVERY_FAILED_STATE);
    assert_eq!(result.report.retry_count, 0);
    assert_eq!(result.report.repair_attempts, 0);
    assert_eq!(result.report.used_repair_bytes, 0);
    assert!(!written.contains("recovered=true"));
    assert!(events.iter().any(|event| {
        event.event_type == EventType::RecoveryStopped
            && event.payload["stop_reason"] == "repair_budget_exceeded"
    }));

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

fn fixture_repo() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = std::env::temp_dir().join(format!(
        "coding-agent-harness-runtime-test-{}-{suffix}",
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

fn write_recovery_fixture_project(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write_file(root, "AGENTS.md", "Use deterministic recovery fixtures.")?;
    write_file(
        root,
        "Cargo.toml",
        "[package]\nname = \"recovery-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )?;
    write_file(
        root,
        "src/lib.rs",
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn fake_patch_contains_recovery_marker() {\n        let content = std::fs::read_to_string(\".harness/fake-agent-turn.md\")\n            .expect(\"fake model patch should exist\");\n        assert!(content.contains(\"recovered=true\"), \"missing recovery marker\");\n    }\n}\n",
    )?;
    Ok(())
}

fn recovery_fixture_verification() -> VerificationCommandRequest {
    VerificationCommandRequest::new("cargo", vec!["test".to_owned(), "--quiet".to_owned()])
}
