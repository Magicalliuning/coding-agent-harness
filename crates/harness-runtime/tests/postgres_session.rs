use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_db::PostgresEventStore;
use harness_events::{EventType, NewEvent, ToolCallIntentPayload};
use harness_policy::PolicyDecision;
use harness_runtime::{
    CodexCliAvailabilityRequest, CodexWorkerLaneBudget, CodexWorkerLaneFixture,
    CodexWorkerLaneRequest, CodexWorkerLaneWorkspace, CodexWorkerSubprocess, CodexWorkerUsage,
    ContextBudget, FakeModelTurnRequest, PENDING_COMMIT_APPROVAL_STATE, RECOVERY_FAILED_STATE,
    RecoveryStopReason, Runtime, SelfRecoveryLoopRequest, SessionContextCompileRequest,
    SessionStatus, SmallCodingTaskRequest, StartSessionRequest, UsageConfidence,
    VerificationCommandRequest, WorkerLaneStatus, detect_codex_cli_availability,
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
fn concurrent_event_appends_keep_session_sequence_contiguous()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("concurrent-eventlog-fixture"))?;
    let worker_count = 12;
    let barrier = Arc::new(Barrier::new(worker_count));
    let mut handles = Vec::new();

    for index in 0..worker_count {
        let barrier = Arc::clone(&barrier);
        let database_url = database_url.clone();
        let session_id = started.session_id;

        handles.push(thread::spawn(move || -> Result<(), String> {
            let store =
                PostgresEventStore::connect(&database_url).map_err(|error| error.to_string())?;
            let event = NewEvent::tool_call_intended(
                session_id,
                ToolCallIntentPayload::new(
                    "verify_command",
                    "cargo",
                    vec![format!("--worker={index}")],
                    "concurrent-eventlog-fixture",
                ),
            )
            .map_err(|error| error.to_string())?;

            barrier.wait();
            store
                .append_event(event)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }));
    }

    for handle in handles {
        handle
            .join()
            .map_err(|_| std::io::Error::other("append thread panicked"))?
            .map_err(std::io::Error::other)?;
    }

    let events = runtime.events_for_session(started.session_id)?;
    let sequences = events
        .iter()
        .map(|event| event.sequence)
        .collect::<Vec<_>>();
    let expected = (1..=worker_count + 1)
        .map(|sequence| sequence as i64)
        .collect::<Vec<_>>();

    assert_eq!(events.len(), worker_count + 1);
    assert_eq!(sequences, expected);

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
fn pending_approval_projection_shows_pending_diff() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = fixture_repo()?;
    write_file(&repo, "AGENTS.md", "Use deterministic agent fixtures.")?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;

    runtime.run_small_coding_task(
        started.session_id,
        SmallCodingTaskRequest {
            task: "write the approval projection fake patch".to_owned(),
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

    let approval = runtime.show_approval(started.session_id)?;

    assert_eq!(approval.state, PENDING_COMMIT_APPROVAL_STATE);
    assert_eq!(approval.diff.files_changed, 1);
    assert_eq!(approval.diff.paths, vec![".harness/fake-agent-turn.md"]);
    assert_eq!(
        approval.summary,
        "verification passed; awaiting human commit approval"
    );

    Ok(())
}

#[test]
fn approving_pending_diff_records_approved_projection() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let session_id = pending_approval_session(&mut runtime)?;

    let approval = runtime.approve_pending_diff(session_id)?;
    let events = runtime.events_for_session(session_id)?;

    assert_eq!(approval.state, "approved");
    assert_eq!(approval.rejection_reason, None);
    assert_eq!(approval.event_count, events.len());
    assert_eq!(
        events.last().expect("last event").event_type,
        EventType::CommitApproved
    );
    assert_eq!(
        events.last().expect("last event").payload["actor"],
        "runtime"
    );

    Ok(())
}

#[test]
fn rejecting_pending_diff_records_reason_projection() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let session_id = pending_approval_session(&mut runtime)?;

    let approval = runtime.reject_pending_diff(session_id, "not the intended change")?;
    let events = runtime.events_for_session(session_id)?;

    assert_eq!(approval.state, "rejected");
    assert_eq!(
        approval.rejection_reason.as_deref(),
        Some("not the intended change")
    );
    assert_eq!(approval.event_count, events.len());
    assert_eq!(
        events.last().expect("last event").event_type,
        EventType::CommitRejected
    );
    assert_eq!(
        events.last().expect("last event").payload["reason"],
        "not the intended change"
    );
    assert_eq!(
        events.last().expect("last event").payload["actor"],
        "runtime"
    );

    Ok(())
}

#[test]
fn invalid_or_duplicate_approval_actions_do_not_corrupt_state()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let session_id = pending_approval_session(&mut runtime)?;

    let blank_rejection = runtime
        .reject_pending_diff(session_id, "  ")
        .expect_err("blank rejection reason should fail");
    let event_count_before_approval = runtime.events_for_session(session_id)?.len();
    assert!(
        blank_rejection
            .to_string()
            .contains("rejection reason is required")
    );

    runtime.approve_pending_diff(session_id)?;
    let event_count_after_approval = runtime.events_for_session(session_id)?.len();

    let duplicate_approval = runtime
        .approve_pending_diff(session_id)
        .expect_err("duplicate approval should fail");
    let rejected_after_approval = runtime
        .reject_pending_diff(session_id, "too late")
        .expect_err("reject after approval should fail");
    let approval = runtime.show_approval(session_id)?;
    let final_event_count = runtime.events_for_session(session_id)?.len();

    assert_eq!(event_count_after_approval, event_count_before_approval + 1);
    assert!(duplicate_approval.to_string().contains("already approved"));
    assert!(
        rejected_after_approval
            .to_string()
            .contains("already approved")
    );
    assert_eq!(approval.state, "approved");
    assert_eq!(final_event_count, event_count_after_approval);

    Ok(())
}

#[test]
fn approval_actions_require_recorded_pending_diff() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = fixture_repo()?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;

    let show_error = runtime
        .show_approval(started.session_id)
        .expect_err("missing pending approval should fail");
    let approve_error = runtime
        .approve_pending_diff(started.session_id)
        .expect_err("missing pending approval should not approve");
    let reject_error = runtime
        .reject_pending_diff(started.session_id, "no diff")
        .expect_err("missing pending approval should not reject");
    let events = runtime.events_for_session(started.session_id)?;

    assert!(
        show_error
            .to_string()
            .contains("pending diff was not recorded")
    );
    assert!(
        approve_error
            .to_string()
            .contains("pending diff was not recorded")
    );
    assert!(
        reject_error
            .to_string()
            .contains("pending diff was not recorded")
    );
    assert_eq!(events.len(), 1);

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
fn codex_worker_lane_records_output_usage_and_state_events()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = git_fixture_repo()?;
    let repo_path = repo.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path.clone()))?;

    let mut request = CodexWorkerLaneRequest::new(
        "draft the next harness patch",
        CodexWorkerLaneFixture {
            status: WorkerLaneStatus::Succeeded,
            exit_code: Some(0),
            stdout: "codex proposed a patch".to_owned(),
            stderr: String::new(),
            duration_ms: 42,
            usage: CodexWorkerUsage {
                prompt_tokens: Some(120),
                completion_tokens: Some(40),
                confidence: UsageConfidence::LocalEstimate,
            },
        },
    );
    request.budget = CodexWorkerLaneBudget {
        max_prompt_tokens: 8_192,
        max_output_tokens: 2_048,
        max_stdout_bytes: 64 * 1024,
    };
    let result = runtime.run_codex_worker_lane(started.session_id, request)?;
    let observation = result.observation.as_ref().expect("worker observation");
    let events = runtime.events_for_session(started.session_id)?;
    let event_types = events
        .iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();

    assert_eq!(result.decision, PolicyDecision::Allow);
    assert_eq!(result.final_status, WorkerLaneStatus::Succeeded);
    assert_eq!(observation.stdout, "codex proposed a patch");
    assert_eq!(observation.usage.confidence, UsageConfidence::LocalEstimate);
    assert_eq!(
        result.event_replay.last_event_type.as_deref(),
        Some("worker_lane.state_changed")
    );
    assert_eq!(
        event_types,
        vec![
            EventType::SessionStarted,
            EventType::WorkerLaneRequested,
            EventType::PolicyDecided,
            EventType::WorkerLaneWorktreeAllocated,
            EventType::WorkerLaneStateChanged,
            EventType::WorkerLaneStateChanged,
            EventType::WorkerLaneObservationRecorded,
            EventType::WorkerLaneStateChanged,
        ]
    );
    assert_eq!(events[1].payload["lane_kind"], "codex_cli");
    assert_eq!(events[1].payload["workspace_path"], repo_path);
    assert_eq!(events[1].payload["worktree_path"], serde_json::Value::Null);
    assert_ne!(events[3].payload["worktree_path"], repo_path);
    assert_eq!(
        events[3].payload["session_repo_path"],
        events[1].payload["workspace_path"]
    );
    assert_eq!(events[3].payload["base_ref"], "HEAD");
    assert!(
        events[3].payload["worktree_path"]
            .as_str()
            .is_some_and(|path| Path::new(path).exists())
    );
    assert_eq!(events[1].payload["budget"]["max_output_tokens"], 2_048);
    assert_eq!(events[6].payload["usage_confidence"], "local_estimate");
    assert_eq!(events[7].payload["to_state"], "succeeded");

    Ok(())
}

#[test]
fn codex_worker_lane_policy_rejection_skips_worker_observation()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = fixture_repo()?;
    let repo_path = repo.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path.clone()))?;

    let mut request = CodexWorkerLaneRequest::new(
        String::new(),
        CodexWorkerLaneFixture::succeeded("should not run"),
    );
    request.budget = CodexWorkerLaneBudget {
        max_prompt_tokens: 8_192,
        max_output_tokens: 2_048,
        max_stdout_bytes: 64 * 1024,
    };
    let result = runtime.run_codex_worker_lane(started.session_id, request)?;
    let events = runtime.events_for_session(started.session_id)?;

    assert_eq!(result.decision, PolicyDecision::Deny);
    assert_eq!(result.final_status, WorkerLaneStatus::Rejected);
    assert_eq!(result.observation, None);
    assert_eq!(events.len(), 4);
    assert_eq!(events[2].event_type, EventType::PolicyDecided);
    assert_eq!(events[2].payload["decision"], "deny");
    assert_eq!(events[3].event_type, EventType::WorkerLaneStateChanged);
    assert_eq!(events[3].payload["to_state"], "rejected");

    Ok(())
}

#[test]
fn codex_worker_lane_marks_timeout_after_captured_fixture_output()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = git_fixture_repo()?;
    let repo_path = repo.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path.clone()))?;

    let mut request = CodexWorkerLaneRequest::new(
        "draft the next harness patch",
        CodexWorkerLaneFixture {
            status: WorkerLaneStatus::Succeeded,
            exit_code: Some(0),
            stdout: "late output".to_owned(),
            stderr: String::new(),
            duration_ms: 11,
            usage: CodexWorkerUsage::unknown(),
        },
    );
    request.timeout_ms = 10;
    request.budget = CodexWorkerLaneBudget {
        max_prompt_tokens: 8_192,
        max_output_tokens: 2_048,
        max_stdout_bytes: 64 * 1024,
    };
    let result = runtime.run_codex_worker_lane(started.session_id, request)?;

    assert_eq!(result.final_status, WorkerLaneStatus::TimedOut);
    assert_eq!(
        result.observation.as_ref().map(|item| item.status),
        Some(WorkerLaneStatus::TimedOut)
    );
    assert_eq!(
        result.event_replay.last_event_type.as_deref(),
        Some("worker_lane.state_changed")
    );

    Ok(())
}

#[test]
fn codex_worker_lane_honors_pre_start_cancellation() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = git_fixture_repo()?;
    let repo_path = repo.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path.clone()))?;

    let mut request = CodexWorkerLaneRequest::new(
        "draft the next harness patch",
        CodexWorkerLaneFixture::succeeded("should not run"),
    );
    request.cancellation_requested = true;
    request.budget = CodexWorkerLaneBudget {
        max_prompt_tokens: 8_192,
        max_output_tokens: 2_048,
        max_stdout_bytes: 64 * 1024,
    };
    let result = runtime.run_codex_worker_lane(started.session_id, request)?;
    let events = runtime.events_for_session(started.session_id)?;

    assert_eq!(result.final_status, WorkerLaneStatus::Cancelled);
    assert_eq!(
        result.observation.as_ref().map(|item| item.status),
        Some(WorkerLaneStatus::Cancelled)
    );
    assert_eq!(
        events.last().expect("last event").payload["to_state"],
        "cancelled"
    );

    Ok(())
}

#[test]
fn codex_worker_lane_records_task_worktree_allocation_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = fixture_repo()?;
    let repo_path = repo.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path.clone()))?;

    let result = runtime.run_codex_worker_lane(
        started.session_id,
        CodexWorkerLaneRequest::new(
            "draft the next harness patch",
            CodexWorkerLaneFixture::succeeded("should not run"),
        ),
    )?;
    let events = runtime.events_for_session(started.session_id)?;

    assert_eq!(result.final_status, WorkerLaneStatus::Failed);
    assert_eq!(result.observation, None);
    assert!(result.reason.contains("task worktree allocation failed"));
    assert_eq!(events[1].payload["workspace_path"], repo_path);
    assert_eq!(events[1].payload["worktree_path"], serde_json::Value::Null);
    assert_eq!(
        events.last().expect("last event").payload["to_state"],
        "failed"
    );

    Ok(())
}

#[test]
fn codex_worker_lane_uses_current_workspace_only_when_explicit()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = fixture_repo()?;
    let repo_path = repo.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path.clone()))?;

    let mut request = CodexWorkerLaneRequest::new(
        "draft directly in the current workspace",
        CodexWorkerLaneFixture::succeeded("current workspace run"),
    );
    request.workspace = CodexWorkerLaneWorkspace::dangerous_current_workspace();
    let result = runtime.run_codex_worker_lane(started.session_id, request)?;
    let events = runtime.events_for_session(started.session_id)?;

    assert_eq!(result.decision, PolicyDecision::Allow);
    assert_eq!(result.final_status, WorkerLaneStatus::Succeeded);
    assert_eq!(events[1].payload["workspace_path"], repo_path);
    assert_eq!(events[1].payload["worktree_path"], serde_json::Value::Null);

    Ok(())
}

#[test]
fn codex_worker_lane_runs_fake_subprocess_and_records_diff_pending_approval()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let (script_name, script) = fake_success_runner_script();
    let repo = git_fixture_repo_with_files(&[(script_name, script)])?;
    let repo_path = repo.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path.clone()))?;

    let mut request = CodexWorkerLaneRequest::new_subprocess(
        "subprocess env task",
        fake_runner_command(script_name).with_env("HARNESS_CUSTOM_MARKER", "custom-env"),
    );
    request.budget = CodexWorkerLaneBudget {
        max_prompt_tokens: 8_192,
        max_output_tokens: 2_048,
        max_stdout_bytes: 4,
    };

    let result = runtime.run_codex_worker_lane(started.session_id, request)?;
    let observation = result.observation.as_ref().expect("worker observation");
    let events = runtime.events_for_session(started.session_id)?;
    let worktree_path = events
        .iter()
        .find(|event| event.event_type == EventType::WorkerLaneWorktreeAllocated)
        .and_then(|event| event.payload["worktree_path"].as_str())
        .expect("allocated worktree path");
    let diff_event = events
        .iter()
        .find(|event| event.event_type == EventType::DiffRecorded)
        .expect("diff event");

    assert_eq!(result.final_status, WorkerLaneStatus::Succeeded);
    assert_eq!(
        result.pending_commit_state.as_deref(),
        Some(PENDING_COMMIT_APPROVAL_STATE)
    );
    assert_eq!(
        result.event_replay.last_event_type.as_deref(),
        Some("commit.approval_pending")
    );
    assert_eq!(observation.exit_code, Some(0));
    assert_eq!(observation.stdout, "0123");
    assert_eq!(observation.stderr, "abcd");
    let env_file = fs::read_to_string(Path::new(worktree_path).join("env-task.txt"))?;
    assert!(env_file.contains("subprocess env task"));
    assert!(env_file.contains("custom-env"));
    assert_eq!(git_text(&repo, &["status", "--short"])?, "");
    assert_eq!(diff_event.payload["files_changed"], 1);
    assert_eq!(diff_event.payload["paths"][0], "README.md");
    assert!(
        diff_event.payload["git_status"]
            .as_str()
            .is_some_and(|status| status.contains("README.md"))
    );
    assert!(
        diff_event.payload["git_diff"]
            .as_str()
            .is_some_and(|diff| diff.contains("README.md"))
    );
    assert_eq!(
        events.last().expect("last event").event_type,
        EventType::CommitApprovalPending
    );

    Ok(())
}

#[test]
fn codex_worker_lane_fake_subprocess_reports_non_zero_exit()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let (script_name, script) = fake_failure_runner_script();
    let repo = git_fixture_repo_with_files(&[(script_name, script)])?;
    let repo_path = repo.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path))?;

    let result = runtime.run_codex_worker_lane(
        started.session_id,
        CodexWorkerLaneRequest::new_subprocess(
            "failing subprocess task",
            fake_runner_command(script_name),
        ),
    )?;
    let observation = result.observation.as_ref().expect("worker observation");

    assert_eq!(result.final_status, WorkerLaneStatus::Failed);
    assert_eq!(result.pending_commit_state, None);
    assert_eq!(observation.exit_code, Some(7));
    assert!(observation.stdout.contains("failure stdout"));
    assert!(observation.stderr.contains("failure stderr"));

    Ok(())
}

#[test]
fn codex_worker_lane_fake_subprocess_reports_timeout() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let (script_name, script) = fake_timeout_runner_script();
    let repo = git_fixture_repo_with_files(&[(script_name, script)])?;
    let repo_path = repo.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path))?;

    let mut request = CodexWorkerLaneRequest::new_subprocess(
        "timeout subprocess task",
        fake_runner_command(script_name),
    );
    request.timeout_ms = 20;
    let result = runtime.run_codex_worker_lane(started.session_id, request)?;
    let observation = result.observation.as_ref().expect("worker observation");

    assert_eq!(result.final_status, WorkerLaneStatus::TimedOut);
    assert_eq!(observation.exit_code, None);
    assert!(observation.duration_ms >= 20);
    assert_eq!(result.pending_commit_state, None);

    Ok(())
}

#[test]
fn codex_worker_lane_fake_subprocess_honors_pre_start_cancellation()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = git_fixture_repo()?;
    let repo_path = repo.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path))?;

    let mut request = CodexWorkerLaneRequest::new_subprocess(
        "cancelled subprocess task",
        CodexWorkerSubprocess::new("missing-runner-should-not-start", Vec::new()),
    );
    request.cancellation_requested = true;
    let result = runtime.run_codex_worker_lane(started.session_id, request)?;
    let observation = result.observation.as_ref().expect("worker observation");

    assert_eq!(result.final_status, WorkerLaneStatus::Cancelled);
    assert_eq!(observation.exit_code, None);
    assert!(observation.stderr.contains("cancelled before start"));

    Ok(())
}

#[test]
fn codex_worker_lane_fake_subprocess_reports_missing_executable()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = git_fixture_repo()?;
    let repo_path = repo.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path))?;

    let result = runtime.run_codex_worker_lane(
        started.session_id,
        CodexWorkerLaneRequest::new_subprocess(
            "missing executable subprocess task",
            CodexWorkerSubprocess::new("missing-coding-agent-harness-runner", Vec::new()),
        ),
    )?;
    let observation = result.observation.as_ref().expect("worker observation");

    assert_eq!(result.final_status, WorkerLaneStatus::Failed);
    assert_eq!(observation.exit_code, None);
    assert!(observation.stderr.contains("worker executable unavailable"));
    assert_eq!(result.pending_commit_state, None);

    Ok(())
}

#[test]
fn codex_cli_availability_reports_missing_executable() {
    let availability = detect_codex_cli_availability(&CodexCliAvailabilityRequest {
        program: "missing-codex-cli-for-harness-test".to_owned(),
        codex_home: None,
        codex_api_key_present: false,
    });

    assert!(!availability.available);
    assert!(!availability.authenticated);
    assert!(
        availability
            .skipped_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("could not be started"))
    );
}

#[test]
fn codex_cli_availability_reports_unauthenticated_skip() -> Result<(), Box<dyn std::error::Error>> {
    let codex_home = fixture_repo()?;
    let availability = detect_codex_cli_availability(&CodexCliAvailabilityRequest {
        program: "cargo".to_owned(),
        codex_home: Some(codex_home),
        codex_api_key_present: false,
    });

    assert!(availability.available);
    assert!(!availability.authenticated);
    assert!(
        availability
            .version
            .as_deref()
            .is_some_and(|version| version.contains("cargo"))
    );
    assert!(
        availability
            .skipped_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("authentication was not detected"))
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

fn fixture_repo() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = std::env::temp_dir().join(format!(
        "coding-agent-harness-runtime-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn git_fixture_repo() -> Result<PathBuf, Box<dyn std::error::Error>> {
    git_fixture_repo_with_files(&[])
}

fn git_fixture_repo_with_files(
    files: &[(&str, &str)],
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let repo = fixture_repo()?;
    write_file(&repo, "README.md", "fixture repository")?;
    for (relative_path, content) in files {
        write_file(&repo, relative_path, content)?;
    }
    run_git(&repo, &["init"])?;
    run_git(&repo, &["add", "."])?;
    run_git(
        &repo,
        &[
            "-c",
            "user.name=Coding Agent Harness Test",
            "-c",
            "user.email=harness-test@example.invalid",
            "commit",
            "-m",
            "initial fixture",
        ],
    )?;
    Ok(repo)
}

fn pending_approval_session(runtime: &mut Runtime) -> Result<Uuid, Box<dyn std::error::Error>> {
    let repo = fixture_repo()?;
    write_file(&repo, "AGENTS.md", "Use deterministic agent fixtures.")?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;

    runtime.run_small_coding_task(
        started.session_id,
        SmallCodingTaskRequest {
            task: "write the approval state fake patch".to_owned(),
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

    Ok(started.session_id)
}

fn fake_runner_command(script_name: &str) -> CodexWorkerSubprocess {
    #[cfg(windows)]
    {
        CodexWorkerSubprocess::new("cmd", vec!["/C".to_owned(), script_name.to_owned()])
    }
    #[cfg(not(windows))]
    {
        CodexWorkerSubprocess::new("sh", vec![script_name.to_owned()])
    }
}

#[cfg(windows)]
fn fake_success_runner_script() -> (&'static str, &'static str) {
    (
        "fake-success-runner.cmd",
        "@echo off\r\necho 0123456789\r\necho abcdefghij 1>&2\r\necho env:%HARNESS_WORKER_TASK%> env-task.txt\r\necho custom:%HARNESS_CUSTOM_MARKER%>> env-task.txt\r\necho changed by fake runner>> README.md\r\nexit /B 0\r\n",
    )
}

#[cfg(not(windows))]
fn fake_success_runner_script() -> (&'static str, &'static str) {
    (
        "fake-success-runner.sh",
        "#!/bin/sh\nprintf '0123456789\\n'\nprintf 'abcdefghij\\n' >&2\nprintf 'env:%s\\n' \"$HARNESS_WORKER_TASK\" > env-task.txt\nprintf 'custom:%s\\n' \"$HARNESS_CUSTOM_MARKER\" >> env-task.txt\nprintf 'changed by fake runner\\n' >> README.md\nexit 0\n",
    )
}

#[cfg(windows)]
fn fake_failure_runner_script() -> (&'static str, &'static str) {
    (
        "fake-failure-runner.cmd",
        "@echo off\r\necho failure stdout\r\necho failure stderr 1>&2\r\nexit /B 7\r\n",
    )
}

#[cfg(not(windows))]
fn fake_failure_runner_script() -> (&'static str, &'static str) {
    (
        "fake-failure-runner.sh",
        "#!/bin/sh\nprintf 'failure stdout\\n'\nprintf 'failure stderr\\n' >&2\nexit 7\n",
    )
}

#[cfg(windows)]
fn fake_timeout_runner_script() -> (&'static str, &'static str) {
    (
        "fake-timeout-runner.cmd",
        "@echo off\r\necho starting timeout\r\nping -n 3 127.0.0.1 >NUL\r\necho should not finish\r\nexit /B 0\r\n",
    )
}

#[cfg(not(windows))]
fn fake_timeout_runner_script() -> (&'static str, &'static str) {
    (
        "fake-timeout-runner.sh",
        "#!/bin/sh\nprintf 'starting timeout\\n'\nsleep 2\nprintf 'should not finish\\n'\nexit 0\n",
    )
}

fn run_git(root: &Path, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "git {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(())
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "git {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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
