use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_core::HarnessError;
use harness_db::PostgresEventStore;
use harness_events::{
    CommitApprovalDecisionPayload, CommitApprovalPendingPayload, CommitHandoffPayload,
    DiffSummaryPayload, EventType, NewEvent, TaskLeasePayload, TaskQueuePayload,
    ToolCallIntentPayload, WorkerLaneBudgetPayload, WorkerLaneObservationPayload,
    WorkerLaneRequestPayload, WorkerLaneWorktreeAllocatedPayload,
};
use harness_policy::PolicyDecision;
use harness_runtime::{
    ApprovalCommitInspectRequest, COMMIT_APPROVED_STATE, COMMIT_FAILED_STATE,
    COMMIT_REJECTED_STATE, COMMITTED_STATE, COMMITTING_STATE, CodexCliAvailabilityRequest,
    CodexWorkerLaneBudget, CodexWorkerLaneFixture, CodexWorkerLaneRequest,
    CodexWorkerLaneWorkspace, CodexWorkerSubprocess, CodexWorkerUsage, ContextBudget,
    CreateTaskRequest, DEFAULT_TASK_WORKER_LANE_KIND, EventTimelineInspectRequest,
    FakeModelTurnRequest, HeartbeatTaskLeaseRequest, LeaseNextTaskRequest,
    LeasedCodexWorkerTaskRequest, PENDING_COMMIT_APPROVAL_STATE, RECOVERY_FAILED_STATE,
    RecoveryStopReason, Runtime, SelfRecoveryLoopRequest, SessionContextCompileRequest,
    SessionStatus, SmallCodingTaskRequest, StartSessionRequest, TASK_CANCELLED_STATE,
    TASK_COMPLETED_STATE, TASK_FAILED_STATE, TASK_LEASED_STATE, TASK_QUEUED_STATE,
    TASK_RETRY_QUEUED_STATE, TASK_STOP_REASON_LEASE_EXPIRED, TASK_STOP_REASON_MAX_RETRIES_EXCEEDED,
    TASK_STOPPED_STATE, TaskQueueInspectRequest, UsageConfidence, VerificationCommandRequest,
    WorkerLaneDiffInspectRequest, WorkerLaneStatus, detect_codex_cli_availability,
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
fn task_create_list_show_projects_first_class_tasks() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo_path = format!("task-fixture-repo-{}", Uuid::new_v4());
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path.clone()))?;
    let first = runtime.create_task(
        started.session_id,
        CreateTaskRequest {
            input: "write the first runtime task".to_owned(),
            repo_path: None,
            worker_lane_kind: "codex_cli_worker_lane".to_owned(),
            context: SessionContextCompileRequest {
                budget: ContextBudget {
                    max_bytes: 4096,
                    max_files: 8,
                    max_skill_files: 2,
                },
                focus_terms: vec!["agent".to_owned(), "task".to_owned()],
            },
            max_output_tokens: 384,
        },
    )?;
    let second = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("write the second runtime task"),
    )?;
    let tasks = runtime.list_tasks(started.session_id)?;
    let shown = runtime.show_task(started.session_id, first.task_id)?;

    assert_ne!(first.task_id, second.task_id);
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].task_id, first.task_id);
    assert_eq!(tasks[1].task_id, second.task_id);
    assert_eq!(shown.session_id, started.session_id);
    assert_eq!(shown.repo_path, repo_path);
    assert_eq!(shown.input, "write the first runtime task");
    assert_eq!(shown.status, "created");
    assert_eq!(shown.worker_lane_kind, "codex_cli_worker_lane");
    assert_eq!(shown.context_budget.max_bytes, 4096);
    assert_eq!(shown.context_budget.max_files, 8);
    assert_eq!(shown.context_budget.max_skill_files, 2);
    assert_eq!(
        shown.focus_terms,
        vec!["agent".to_owned(), "task".to_owned()]
    );
    assert_eq!(shown.max_output_tokens, 384);
    assert_eq!(shown.worktree, None);
    assert_eq!(shown.worker_output, None);
    assert_eq!(shown.diff, None);
    assert_eq!(shown.approval, None);
    assert_eq!(shown.commit, None);
    assert_eq!(shown.event_count, 3);

    Ok(())
}

#[test]
fn session_inspect_report_summarizes_session_without_mutating_state()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo_path = format!("inspect-fixture-repo-{}", Uuid::new_v4());
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path.clone()))?;
    let first = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("write the first inspect task"),
    )?;
    let second = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("write the second inspect task"),
    )?;
    runtime.enqueue_task(started.session_id, first.task_id)?;

    let events_before = runtime.events_for_session(started.session_id)?;
    let queue_before = PostgresEventStore::connect(&database_url)?
        .task_queue_record(first.task_id)?
        .expect("task queue record");
    let report = runtime.inspect_session(started.session_id)?;
    let events_after = runtime.events_for_session(started.session_id)?;
    let queue_after = PostgresEventStore::connect(&database_url)?
        .task_queue_record(first.task_id)?
        .expect("task queue record after inspect");

    assert_eq!(report.session_id, started.session_id);
    assert_eq!(report.repo_path, repo_path);
    assert_eq!(report.status, "started");
    assert_eq!(report.event_count, events_before.len());
    assert_eq!(report.latest_event_sequence, Some(4));
    assert_eq!(report.latest_event_type.as_deref(), Some("task.enqueued"));
    assert_eq!(report.task_count, 2);
    assert_eq!(
        report
            .task_status_counts
            .iter()
            .find(|count| count.status == "created")
            .map(|count| count.count),
        Some(1)
    );
    assert_eq!(
        report
            .task_status_counts
            .iter()
            .find(|count| count.status == TASK_QUEUED_STATE)
            .map(|count| count.count),
        Some(1)
    );
    assert_eq!(report.source_of_truth, "EventLog");
    assert_eq!(report.projection_kind, "derived_from_eventlog");
    assert_eq!(events_after.len(), events_before.len());
    assert_eq!(queue_after, queue_before);
    assert_eq!(
        runtime
            .show_task(started.session_id, second.task_id)?
            .status,
        "created"
    );

    Ok(())
}

#[test]
fn event_timeline_report_filters_task_scope_and_bounds_payloads()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("timeline-fixture-repo"))?;
    let first = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("write the first timeline task"),
    )?;
    let second = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("write the second timeline task"),
    )?;
    runtime.enqueue_task(started.session_id, first.task_id)?;
    PostgresEventStore::connect(&database_url)?.append_event(NewEvent::diff_recorded(
        started.session_id,
        DiffSummaryPayload::new(1, 1, 0, vec!["README.md".to_owned()])
            .with_task_id(first.task_id)
            .with_git_evidence(
                " M README.md",
                format!("diff --git a/README.md b/README.md\n{}", "x".repeat(512)),
            ),
    )?)?;

    let events_before = runtime.events_for_session(started.session_id)?;
    let session_timeline = runtime.inspect_event_timeline(
        EventTimelineInspectRequest::new(started.session_id).with_payload_limit_bytes(64),
    )?;
    let task_timeline = runtime.inspect_event_timeline(
        EventTimelineInspectRequest::new(started.session_id)
            .with_task_id(first.task_id)
            .with_payload_limit_bytes(64),
    )?;
    let events_after = runtime.events_for_session(started.session_id)?;

    assert_eq!(session_timeline.session_id, started.session_id);
    assert_eq!(session_timeline.task_id, None);
    assert_eq!(session_timeline.event_count, events_before.len());
    assert_eq!(session_timeline.payload_limit_bytes, 64);
    assert_eq!(session_timeline.events[0].sequence, 1);
    assert_eq!(session_timeline.events[0].event_type, "session.started");
    assert_eq!(session_timeline.events[0].schema_version, 1);
    assert_eq!(session_timeline.events[0].task_id, None);
    assert_eq!(session_timeline.source_of_truth, "EventLog");
    assert_eq!(
        session_timeline.projection_kind,
        "bounded_eventlog_timeline"
    );
    assert!(
        session_timeline
            .events
            .iter()
            .any(|event| event.payload.truncated)
    );
    assert!(
        session_timeline
            .events
            .iter()
            .all(|event| event.payload.text.len() <= 64)
    );

    assert_eq!(task_timeline.task_id, Some(first.task_id));
    assert_eq!(task_timeline.event_count, 3);
    assert_eq!(
        task_timeline
            .events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![2, 4, 5]
    );
    assert!(
        task_timeline
            .events
            .iter()
            .all(|event| event.task_id == Some(first.task_id))
    );
    assert!(
        !task_timeline
            .events
            .iter()
            .any(|event| event.task_id == Some(second.task_id))
    );
    assert!(
        task_timeline
            .events
            .iter()
            .any(|event| event.event_type == "diff.recorded" && event.payload.truncated)
    );
    assert_eq!(events_after.len(), events_before.len());

    let missing = runtime
        .inspect_event_timeline(
            EventTimelineInspectRequest::new(started.session_id).with_task_id(Uuid::new_v4()),
        )
        .expect_err("missing task scope should fail");
    assert!(missing.message().contains("task not found"));

    Ok(())
}

#[test]
fn task_inspect_report_combines_projection_queue_and_task_scoped_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("task-inspect-fixture-repo"))?;
    let first = runtime.create_task(
        started.session_id,
        CreateTaskRequest {
            input: "write the inspect task".to_owned(),
            repo_path: None,
            worker_lane_kind: DEFAULT_TASK_WORKER_LANE_KIND.to_owned(),
            context: SessionContextCompileRequest {
                budget: ContextBudget {
                    max_bytes: 4096,
                    max_files: 8,
                    max_skill_files: 2,
                },
                focus_terms: vec!["inspect".to_owned()],
            },
            max_output_tokens: 384,
        },
    )?;
    let unrelated = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("write the unrelated inspect task"),
    )?;
    runtime.enqueue_task_with_max_retries(started.session_id, first.task_id, 2)?;
    runtime
        .lease_next_task(LeaseNextTaskRequest {
            worker_id: "inspect-worker".to_owned(),
            lease_duration_ms: 30_000,
        })?
        .expect("leased task");

    let store = PostgresEventStore::connect(&database_url)?;
    store.append_event(NewEvent::worker_lane_requested(
        started.session_id,
        WorkerLaneRequestPayload {
            task_id: Some(first.task_id),
            lane_id: "lane-inspect".to_owned(),
            lane_kind: DEFAULT_TASK_WORKER_LANE_KIND.to_owned(),
            task: "write the inspect task".to_owned(),
            workspace_path: "C:/workspace".to_owned(),
            worktree_path: Some("C:/requested-worktree".to_owned()),
            timeout_ms: 30_000,
            cancellation_requested: false,
            budget: WorkerLaneBudgetPayload::new(8192, 2048, 65536),
        },
    )?)?;
    store.append_event(NewEvent::worker_lane_worktree_allocated(
        started.session_id,
        WorkerLaneWorktreeAllocatedPayload::new(
            "lane-inspect",
            DEFAULT_TASK_WORKER_LANE_KIND,
            "C:/repo",
            "C:/allocated-worktree",
            "HEAD",
        )
        .with_task_id(first.task_id),
    )?)?;
    store.append_event(NewEvent::diff_recorded(
        started.session_id,
        DiffSummaryPayload::new(1, 2, 0, vec!["README.md".to_owned()])
            .with_task_id(first.task_id)
            .with_repo_path("C:/diff-repo"),
    )?)?;
    store.append_event(NewEvent::commit_approval_pending(
        started.session_id,
        CommitApprovalPendingPayload::new(
            PENDING_COMMIT_APPROVAL_STATE,
            "worker produced reviewable diff",
        )
        .with_task_id(first.task_id),
    )?)?;
    store.append_event(NewEvent::commit_approved(
        started.session_id,
        CommitApprovalDecisionPayload::new(
            COMMIT_APPROVED_STATE,
            "approved for inspect",
            "runtime",
        )
        .with_task_id(first.task_id),
    )?)?;
    store.append_event(NewEvent::commit_succeeded(
        started.session_id,
        CommitHandoffPayload::succeeded(
            "C:/diff-repo",
            "Commit inspect task",
            "runtime",
            "0123456789abcdef0123456789abcdef01234567",
        )
        .with_task_id(first.task_id),
    )?)?;

    let events_before = runtime.events_for_session(started.session_id)?;
    let queue_before = PostgresEventStore::connect(&database_url)?
        .task_queue_record(first.task_id)?
        .expect("task queue record");
    let report = runtime.inspect_task(started.session_id, first.task_id)?;
    let events_after = runtime.events_for_session(started.session_id)?;
    let queue_after = PostgresEventStore::connect(&database_url)?
        .task_queue_record(first.task_id)?
        .expect("task queue record after inspect");

    assert_eq!(report.session_id, started.session_id);
    assert_eq!(report.task_id, first.task_id);
    assert_eq!(report.status, COMMITTED_STATE);
    assert_eq!(report.worker_lane_kind, DEFAULT_TASK_WORKER_LANE_KIND);
    assert_eq!(report.context_budget.max_bytes, 4096);
    assert_eq!(report.context_budget.max_files, 8);
    assert_eq!(report.context_budget.max_skill_files, 2);
    assert_eq!(report.focus_terms, vec!["inspect"]);
    assert_eq!(report.max_output_tokens, 384);
    assert_eq!(
        report.queue.as_ref().expect("queue").status,
        TASK_LEASED_STATE
    );
    assert_eq!(
        report.queue.as_ref().expect("queue").worker_id.as_deref(),
        Some("inspect-worker")
    );
    assert_eq!(report.queue.as_ref().expect("queue").max_retries, Some(2));
    assert_eq!(
        report.lease.as_ref().expect("lease").worker_id,
        "inspect-worker"
    );
    assert_eq!(
        report.approval.as_ref().expect("approval").actor.as_deref(),
        Some("runtime")
    );
    assert_eq!(
        report
            .approval
            .as_ref()
            .expect("approval")
            .reason
            .as_deref(),
        Some("approved for inspect")
    );
    assert_eq!(
        report.commit.as_ref().expect("commit").repo_path.as_deref(),
        Some("C:/diff-repo")
    );
    assert_eq!(
        report
            .commit
            .as_ref()
            .expect("commit")
            .commit_sha
            .as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(
        report.workspace.worker_workspace_path.as_deref(),
        Some("C:/workspace")
    );
    assert_eq!(
        report.workspace.worker_worktree_path.as_deref(),
        Some("C:/requested-worktree")
    );
    assert_eq!(
        report.workspace.allocated_worktree_path.as_deref(),
        Some("C:/allocated-worktree")
    );
    assert_eq!(
        report.workspace.diff_repo_path.as_deref(),
        Some("C:/diff-repo")
    );
    assert_eq!(
        report.diff.as_ref().expect("diff").repo_path.as_deref(),
        Some("C:/diff-repo")
    );
    assert_eq!(report.diff.as_ref().expect("diff").files_changed, 1);
    assert_eq!(report.event_summary.first_sequence, Some(2));
    assert_eq!(
        report.event_summary.latest_event_type.as_deref(),
        Some("commit.succeeded")
    );
    assert!(report.event_summary.event_count < events_before.len());
    assert_eq!(report.source_of_truth, "EventLog");
    assert_eq!(report.projection_kind, "task_inspect_report");
    assert_eq!(events_after.len(), events_before.len());
    assert_eq!(queue_after, queue_before);

    let other_session =
        runtime.start_session(StartSessionRequest::new("other-task-inspect-repo"))?;
    let missing = runtime
        .inspect_task(other_session.session_id, unrelated.task_id)
        .expect_err("task from another session should not inspect");
    assert!(missing.message().contains("task not found"));

    Ok(())
}

#[test]
fn task_inspect_report_exposes_rejected_and_commit_failed_terminal_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("task-inspect-terminal-repo"))?;
    let rejected = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("write the rejected inspect task"),
    )?;
    let failed = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("write the commit failed inspect task"),
    )?;
    let store = PostgresEventStore::connect(&database_url)?;

    store.append_event(NewEvent::diff_recorded(
        started.session_id,
        DiffSummaryPayload::new(1, 1, 0, vec!["rejected.md".to_owned()])
            .with_task_id(rejected.task_id),
    )?)?;
    store.append_event(NewEvent::commit_approval_pending(
        started.session_id,
        CommitApprovalPendingPayload::new(PENDING_COMMIT_APPROVAL_STATE, "rejected task diff")
            .with_task_id(rejected.task_id),
    )?)?;
    store.append_event(NewEvent::commit_rejected(
        started.session_id,
        CommitApprovalDecisionPayload::new(
            COMMIT_REJECTED_STATE,
            "not the intended inspect change",
            "runtime",
        )
        .with_task_id(rejected.task_id),
    )?)?;

    store.append_event(NewEvent::diff_recorded(
        started.session_id,
        DiffSummaryPayload::new(1, 1, 0, vec!["failed.md".to_owned()]).with_task_id(failed.task_id),
    )?)?;
    store.append_event(NewEvent::commit_approval_pending(
        started.session_id,
        CommitApprovalPendingPayload::new(PENDING_COMMIT_APPROVAL_STATE, "failed task diff")
            .with_task_id(failed.task_id),
    )?)?;
    store.append_event(NewEvent::commit_approved(
        started.session_id,
        CommitApprovalDecisionPayload::new(
            COMMIT_APPROVED_STATE,
            "approved for failure",
            "runtime",
        )
        .with_task_id(failed.task_id),
    )?)?;
    store.append_event(NewEvent::commit_failed(
        started.session_id,
        CommitHandoffPayload::failed(
            "C:/failed-repo",
            "Commit failed inspect task",
            "runtime",
            "git failed",
        )
        .with_task_id(failed.task_id),
    )?)?;

    let rejected_report = runtime.inspect_task(started.session_id, rejected.task_id)?;
    let failed_report = runtime.inspect_task(started.session_id, failed.task_id)?;

    let rejected_approval = rejected_report
        .approval
        .as_ref()
        .expect("rejected approval");
    assert_eq!(rejected_report.status, COMMIT_REJECTED_STATE);
    assert_eq!(rejected_approval.actor.as_deref(), Some("runtime"));
    assert_eq!(
        rejected_approval.rejection_reason.as_deref(),
        Some("not the intended inspect change")
    );
    assert_eq!(rejected_report.commit, None);

    let failed_approval = failed_report.approval.as_ref().expect("failed approval");
    let failed_commit = failed_report.commit.as_ref().expect("failed commit");
    assert_eq!(failed_report.status, COMMIT_FAILED_STATE);
    assert_eq!(failed_approval.actor.as_deref(), Some("runtime"));
    assert_eq!(
        failed_approval.reason.as_deref(),
        Some("approved for failure")
    );
    assert_eq!(failed_commit.repo_path.as_deref(), Some("C:/failed-repo"));
    assert_eq!(failed_commit.actor.as_deref(), Some("runtime"));
    assert_eq!(failed_commit.failure_reason.as_deref(), Some("git failed"));

    Ok(())
}

#[test]
fn approval_commit_inspect_report_tracks_pending_approved_and_committed_read_only()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let (script_name, script) = fake_success_runner_script();
    let repo = git_fixture_repo_with_files(&[(script_name, script)])?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;
    let task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write the approval inspect committed task",
    )?;
    runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "approval-inspect-worker",
            CodexWorkerLaneRequest::new_subprocess("placeholder", fake_runner_command(script_name)),
        ))?
        .expect("queued task should run");

    let pending_report = runtime.inspect_approval_commit(
        ApprovalCommitInspectRequest::new(started.session_id).with_task_id(task_id),
    )?;
    assert_eq!(pending_report.scope, "task");
    assert_eq!(pending_report.pending_count, 1);
    assert_eq!(pending_report.decision_count, 0);
    assert_eq!(pending_report.commit_count, 0);
    assert_eq!(pending_report.pending_approvals[0].task_id, Some(task_id));
    assert_eq!(
        pending_report.pending_approvals[0]
            .diff
            .as_ref()
            .expect("pending diff")
            .paths,
        vec!["README.md".to_owned()]
    );
    assert!(
        pending_report.pending_approvals[0]
            .workspace
            .allocated_worktree_path
            .is_some()
    );

    runtime.approve_task_pending_diff(started.session_id, task_id)?;
    let committed = runtime.commit_approved_task_diff(
        started.session_id,
        task_id,
        "Commit approval inspect task",
    )?;
    let store = PostgresEventStore::connect(&database_url)?;
    let events_before = runtime.events_for_session(started.session_id)?;
    let task_before = runtime.show_task(started.session_id, task_id)?;
    let queue_before = store.task_queue_records(Some(started.session_id))?;

    let report = runtime.inspect_approval_commit(
        ApprovalCommitInspectRequest::new(started.session_id).with_task_id(task_id),
    )?;

    assert_eq!(committed.status, COMMITTED_STATE);
    assert_eq!(report.pending_count, 0);
    assert_eq!(report.decision_count, 1);
    assert_eq!(report.commit_count, 2);
    assert_eq!(report.decisions[0].task_id, Some(task_id));
    assert_eq!(report.decisions[0].state, COMMIT_APPROVED_STATE);
    assert_eq!(report.decisions[0].actor.as_deref(), Some("runtime"));
    assert_eq!(
        report.decisions[0].reason,
        "approved by runtime approval state machine"
    );
    assert_eq!(report.commits[0].state, COMMITTING_STATE);
    assert_eq!(report.commits[0].message, "Commit approval inspect task");
    assert_eq!(report.commits[0].actor.as_deref(), Some("runtime"));
    assert_eq!(report.commits[1].state, COMMITTED_STATE);
    assert!(
        report.commits[1]
            .commit_sha
            .as_deref()
            .is_some_and(|sha| sha.len() == 40)
    );
    assert_eq!(report.source_of_truth, "EventLog");
    assert_eq!(report.projection_kind, "approval_commit_inspect_report");

    let events_after = runtime.events_for_session(started.session_id)?;
    let task_after = runtime.show_task(started.session_id, task_id)?;
    let queue_after = store.task_queue_records(Some(started.session_id))?;
    assert_eq!(events_after.len(), events_before.len());
    assert_eq!(task_after, task_before);
    assert_eq!(queue_after, queue_before);

    Ok(())
}

#[test]
fn approval_commit_inspect_report_distinguishes_rejected_and_commit_failed()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let (script_name, script) = fake_success_runner_script();
    let reject_repo = git_fixture_repo_with_files(&[(script_name, script)])?;
    let fail_repo = git_fixture_repo_with_files(&[(script_name, script)])?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;

    let reject_session =
        runtime.start_session(StartSessionRequest::new(reject_repo.display().to_string()))?;
    let reject_task_id = create_and_enqueue_task(
        &mut runtime,
        reject_session.session_id,
        "write the approval inspect rejected task",
    )?;
    runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "approval-reject-worker",
            CodexWorkerLaneRequest::new_subprocess("placeholder", fake_runner_command(script_name)),
        ))?
        .expect("rejected task should run");
    runtime.reject_task_pending_diff(
        reject_session.session_id,
        reject_task_id,
        "not safe for commit",
    )?;

    let rejected = runtime.inspect_approval_commit(
        ApprovalCommitInspectRequest::new(reject_session.session_id).with_task_id(reject_task_id),
    )?;
    assert_eq!(rejected.pending_count, 0);
    assert_eq!(rejected.decision_count, 1);
    assert_eq!(rejected.commit_count, 0);
    assert_eq!(rejected.decisions[0].state, COMMIT_REJECTED_STATE);
    assert_eq!(
        rejected.decisions[0].rejection_reason.as_deref(),
        Some("not safe for commit")
    );

    let fail_session =
        runtime.start_session(StartSessionRequest::new(fail_repo.display().to_string()))?;
    let fail_task_id = create_and_enqueue_task(
        &mut runtime,
        fail_session.session_id,
        "write the approval inspect failed task",
    )?;
    let failed_run = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "approval-fail-worker",
            CodexWorkerLaneRequest::new_subprocess("placeholder", fake_runner_command(script_name)),
        ))?
        .expect("failed task should run");
    let fail_worktree = PathBuf::from(
        failed_run
            .task
            .worktree
            .as_ref()
            .expect("task worktree")
            .worktree_path
            .clone(),
    );
    run_git(&fail_worktree, &["checkout", "--", "README.md"])?;
    runtime.approve_task_pending_diff(fail_session.session_id, fail_task_id)?;
    runtime.commit_approved_task_diff(fail_session.session_id, fail_task_id, "Commit fails")?;

    let failed = runtime.inspect_approval_commit(
        ApprovalCommitInspectRequest::new(fail_session.session_id).with_task_id(fail_task_id),
    )?;
    assert_eq!(failed.pending_count, 0);
    assert_eq!(failed.decision_count, 1);
    assert_eq!(failed.decisions[0].state, COMMIT_APPROVED_STATE);
    assert_eq!(failed.commit_count, 2);
    assert_eq!(failed.commits[0].state, COMMITTING_STATE);
    assert_eq!(failed.commits[1].state, COMMIT_FAILED_STATE);
    assert!(
        failed.commits[1]
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("approved diff has no local changes"))
    );
    assert!(failed.decisions[0].rejection_reason.is_none());

    Ok(())
}

#[test]
fn approval_commit_inspect_report_task_scope_excludes_unrelated_task_events()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("approval-scope-fixture"))?;
    let first = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("write the first approval scoped task"),
    )?;
    let second = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("write the second approval scoped task"),
    )?;
    let store = PostgresEventStore::connect(&database_url)?;
    store.append_event(NewEvent::diff_recorded(
        started.session_id,
        DiffSummaryPayload::new(1, 1, 0, vec!["first.md".to_owned()]).with_task_id(first.task_id),
    )?)?;
    store.append_event(NewEvent::commit_approval_pending(
        started.session_id,
        CommitApprovalPendingPayload::new(PENDING_COMMIT_APPROVAL_STATE, "first pending")
            .with_task_id(first.task_id),
    )?)?;
    store.append_event(NewEvent::diff_recorded(
        started.session_id,
        DiffSummaryPayload::new(1, 1, 0, vec!["second.md".to_owned()]).with_task_id(second.task_id),
    )?)?;
    store.append_event(NewEvent::commit_approval_pending(
        started.session_id,
        CommitApprovalPendingPayload::new(PENDING_COMMIT_APPROVAL_STATE, "second pending")
            .with_task_id(second.task_id),
    )?)?;
    store.append_event(NewEvent::commit_rejected(
        started.session_id,
        CommitApprovalDecisionPayload::new(COMMIT_REJECTED_STATE, "second rejected", "runtime")
            .with_task_id(second.task_id),
    )?)?;

    let scoped = runtime.inspect_approval_commit(
        ApprovalCommitInspectRequest::new(started.session_id).with_task_id(first.task_id),
    )?;
    assert_eq!(scoped.pending_count, 1);
    assert_eq!(scoped.pending_approvals[0].task_id, Some(first.task_id));
    assert_eq!(scoped.pending_approvals[0].summary, "first pending");
    assert_eq!(scoped.decision_count, 0);
    assert_eq!(scoped.commit_count, 0);
    assert!(
        scoped.pending_approvals[0]
            .diff
            .as_ref()
            .expect("first diff")
            .paths
            .contains(&"first.md".to_owned())
    );

    let session_report =
        runtime.inspect_approval_commit(ApprovalCommitInspectRequest::new(started.session_id))?;
    assert_eq!(session_report.pending_count, 1);
    assert_eq!(session_report.decision_count, 1);
    assert_eq!(session_report.decisions[0].task_id, Some(second.task_id));
    assert_eq!(session_report.decisions[0].state, COMMIT_REJECTED_STATE);

    let missing = runtime.inspect_approval_commit(
        ApprovalCommitInspectRequest::new(started.session_id).with_task_id(Uuid::new_v4()),
    );
    assert!(
        missing
            .expect_err("missing task should fail")
            .to_string()
            .contains("task not found")
    );

    Ok(())
}

#[test]
fn task_queue_inspect_report_handles_empty_and_queued_session_scope()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("queue-inspect-empty"))?;

    let empty = runtime.inspect_task_queue(
        TaskQueueInspectRequest::new()
            .with_session_id(started.session_id)
            .with_now_ms(0),
    )?;
    assert_eq!(empty.session_id, Some(started.session_id));
    assert!(empty.empty);
    assert_eq!(empty.total_count, 0);
    assert!(empty.tasks.is_empty());
    assert_eq!(empty.source_of_truth, "task_queue+EventLog");
    assert_eq!(empty.projection_kind, "task_queue_inspect_report");

    let task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write the queued inspect task",
    )?;
    let report = runtime.inspect_task_queue(
        TaskQueueInspectRequest::new()
            .with_session_id(started.session_id)
            .with_now_ms(0),
    )?;

    assert!(!report.empty);
    assert_eq!(report.total_count, 1);
    assert_eq!(report.queue_status_counts[0].status, TASK_QUEUED_STATE);
    assert_eq!(report.queue_status_counts[0].count, 1);
    assert_eq!(report.task_status_counts[0].status, TASK_QUEUED_STATE);
    assert_eq!(report.tasks[0].session_id, started.session_id);
    assert_eq!(report.tasks[0].task_id, task_id);
    assert_eq!(report.tasks[0].queue_status, TASK_QUEUED_STATE);
    assert_eq!(
        report.tasks[0].task_status.as_deref(),
        Some(TASK_QUEUED_STATE)
    );
    assert_eq!(report.tasks[0].status_class, TASK_QUEUED_STATE);
    assert_eq!(report.tasks[0].lease_state, "none");
    assert_eq!(
        report.tasks[0].last_reason.as_deref(),
        Some("task enqueued for worker execution")
    );

    let missing = runtime.inspect_task_queue(
        TaskQueueInspectRequest::new()
            .with_session_id(Uuid::new_v4())
            .with_now_ms(0),
    );
    assert!(
        missing
            .expect_err("missing session should fail")
            .to_string()
            .contains("session not found")
    );

    Ok(())
}

#[test]
fn task_queue_inspect_report_marks_expired_looking_leases_without_mutating()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("queue-inspect-lease"))?;
    let task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write the leased inspect task",
    )?;
    let leased = runtime
        .lease_next_task(LeaseNextTaskRequest {
            worker_id: "queue-inspect-worker".to_owned(),
            lease_duration_ms: 30_000,
        })?
        .expect("task should be leased");
    let lease = leased.lease.as_ref().expect("lease slot");
    let lease_id = lease.lease_id;
    let lease_deadline_ms = lease
        .lease_deadline_ms
        .expect("lease deadline should be recorded");
    let events_before = runtime.events_for_session(started.session_id)?;
    let store = PostgresEventStore::connect(&database_url)?;
    let queue_before = store.task_queue_records(Some(started.session_id))?;

    let active = runtime.inspect_task_queue(
        TaskQueueInspectRequest::new()
            .with_session_id(started.session_id)
            .with_now_ms(0),
    )?;
    assert_eq!(active.active_leased_count, 1);
    assert_eq!(active.expired_looking_leased_count, 0);
    assert_eq!(active.tasks[0].task_id, task_id);
    assert_eq!(active.tasks[0].queue_status, TASK_LEASED_STATE);
    assert_eq!(active.tasks[0].status_class, "active_leased");
    assert_eq!(active.tasks[0].lease_state, "active");
    assert_eq!(
        active.tasks[0].worker_id.as_deref(),
        Some("queue-inspect-worker")
    );
    assert_eq!(active.tasks[0].lease_id, Some(lease_id));
    assert_eq!(active.tasks[0].lease_deadline_ms, Some(lease_deadline_ms));

    let expired_looking = runtime.inspect_task_queue(
        TaskQueueInspectRequest::new()
            .with_session_id(started.session_id)
            .with_now_ms(i64::MAX),
    )?;
    assert_eq!(expired_looking.active_leased_count, 0);
    assert_eq!(expired_looking.expired_looking_leased_count, 1);
    assert_eq!(expired_looking.tasks[0].queue_status, TASK_LEASED_STATE);
    assert_eq!(
        expired_looking.tasks[0].status_class,
        "expired_looking_leased"
    );
    assert_eq!(expired_looking.tasks[0].lease_state, "expired_looking");

    let events_after = runtime.events_for_session(started.session_id)?;
    let queue_after = store.task_queue_records(Some(started.session_id))?;
    assert_eq!(events_after.len(), events_before.len());
    assert_eq!(queue_after, queue_before);

    Ok(())
}

#[test]
fn task_queue_inspect_report_exposes_retry_stopped_and_max_retry_state()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let stopped_session =
        runtime.start_session(StartSessionRequest::new("queue-inspect-stopped"))?;
    let stopped_task_id = create_and_enqueue_task_with_max_retries(
        &mut runtime,
        stopped_session.session_id,
        "write the stopped inspect task",
        0,
    )?;
    runtime
        .lease_next_task(LeaseNextTaskRequest {
            worker_id: "queue-stopped-worker".to_owned(),
            lease_duration_ms: 1,
        })?
        .expect("stopped task should be leased");
    runtime.expire_task_leases(i64::MAX)?;

    let stopped_report = runtime.inspect_task_queue(
        TaskQueueInspectRequest::new()
            .with_session_id(stopped_session.session_id)
            .with_now_ms(0),
    )?;
    assert_eq!(stopped_report.total_count, 1);
    assert_eq!(stopped_report.tasks[0].task_id, stopped_task_id);
    assert_eq!(stopped_report.tasks[0].queue_status, TASK_STOPPED_STATE);
    assert_eq!(
        stopped_report.tasks[0].task_status.as_deref(),
        Some(TASK_STOPPED_STATE)
    );
    assert_eq!(stopped_report.tasks[0].retry_count, 0);
    assert_eq!(stopped_report.tasks[0].max_retries, 0);
    assert_eq!(
        stopped_report.tasks[0].stop_reason.as_deref(),
        Some(TASK_STOP_REASON_MAX_RETRIES_EXCEEDED)
    );

    let retry_session = runtime.start_session(StartSessionRequest::new("queue-inspect-retry"))?;
    let retry_task_id = create_and_enqueue_task_with_max_retries(
        &mut runtime,
        retry_session.session_id,
        "write the retry inspect task",
        2,
    )?;
    runtime
        .lease_next_task(LeaseNextTaskRequest {
            worker_id: "queue-retry-worker".to_owned(),
            lease_duration_ms: 1,
        })?
        .expect("retry task should be leased");
    runtime.expire_task_leases(i64::MAX)?;

    let retry_report = runtime.inspect_task_queue(
        TaskQueueInspectRequest::new()
            .with_session_id(retry_session.session_id)
            .with_now_ms(0),
    )?;
    assert_eq!(retry_report.total_count, 1);
    assert_eq!(retry_report.tasks[0].task_id, retry_task_id);
    assert_eq!(retry_report.tasks[0].queue_status, TASK_RETRY_QUEUED_STATE);
    assert_eq!(
        retry_report.tasks[0].task_status.as_deref(),
        Some(TASK_RETRY_QUEUED_STATE)
    );
    assert_eq!(retry_report.tasks[0].retry_count, 1);
    assert_eq!(retry_report.tasks[0].max_retries, 2);
    assert_eq!(
        retry_report.tasks[0].stop_reason.as_deref(),
        Some(TASK_STOP_REASON_LEASE_EXPIRED)
    );

    Ok(())
}

#[test]
fn task_queue_lease_and_terminal_transitions_are_projected()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("task-queue-fixture"))?;

    let task_id =
        create_and_enqueue_task(&mut runtime, started.session_id, "write the queued task")?;
    let enqueued = runtime.show_task(started.session_id, task_id)?;
    assert_eq!(enqueued.status, TASK_QUEUED_STATE);
    assert_eq!(
        enqueued.queue.as_ref().expect("queue slot").status,
        TASK_QUEUED_STATE
    );

    let leased = runtime
        .lease_next_task(LeaseNextTaskRequest {
            worker_id: "worker-complete".to_owned(),
            lease_duration_ms: 30_000,
        })?
        .expect("queued task should be leased");
    assert_eq!(leased.task_id, task_id);
    assert_eq!(leased.status, TASK_LEASED_STATE);
    assert_eq!(
        leased.queue.as_ref().expect("queue slot").status,
        TASK_LEASED_STATE
    );
    let lease = leased.lease.as_ref().expect("lease slot");
    assert_eq!(lease.worker_id, "worker-complete");
    assert_eq!(lease.status, TASK_LEASED_STATE);
    assert!(lease.lease_deadline_ms.is_some());

    let completed =
        runtime.complete_task_lease(task_id, lease.lease_id, "worker completed task")?;
    assert_eq!(completed.status, TASK_COMPLETED_STATE);
    assert_eq!(
        completed.queue.as_ref().expect("queue slot").status,
        TASK_COMPLETED_STATE
    );
    assert_eq!(
        completed
            .lease
            .as_ref()
            .expect("lease slot")
            .reason
            .as_deref(),
        Some("worker completed task")
    );

    let failed_task_id =
        create_and_enqueue_task(&mut runtime, started.session_id, "write the failed task")?;
    let failed_lease = runtime
        .lease_next_task(LeaseNextTaskRequest::new("worker-fail"))?
        .expect("failed task should be leased")
        .lease
        .expect("failed lease");
    let failed =
        runtime.fail_task_lease(failed_task_id, failed_lease.lease_id, "worker failed task")?;
    assert_eq!(failed.status, TASK_FAILED_STATE);
    assert_eq!(
        failed.lease.as_ref().expect("lease slot").reason.as_deref(),
        Some("worker failed task")
    );

    let cancelled_task_id =
        create_and_enqueue_task(&mut runtime, started.session_id, "write the cancelled task")?;
    let cancelled_lease = runtime
        .lease_next_task(LeaseNextTaskRequest::new("worker-cancel"))?
        .expect("cancelled task should be leased")
        .lease
        .expect("cancelled lease");
    let cancelled = runtime.cancel_task_lease(
        cancelled_task_id,
        cancelled_lease.lease_id,
        "worker cancelled task",
    )?;
    assert_eq!(cancelled.status, TASK_CANCELLED_STATE);
    assert_eq!(
        cancelled
            .lease
            .as_ref()
            .expect("lease slot")
            .reason
            .as_deref(),
        Some("worker cancelled task")
    );

    let events = runtime.events_for_session(started.session_id)?;
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::TaskEnqueued)
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::TaskLeaseAcquired)
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::TaskLeaseCompleted)
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::TaskLeaseFailed)
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::TaskLeaseCancelled)
    );

    Ok(())
}

#[test]
fn concurrent_task_lease_attempts_have_single_winner() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started =
        runtime.start_session(StartSessionRequest::new("concurrent-task-lease-fixture"))?;
    let task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write the concurrently leased task",
    )?;
    let worker_count = 8;
    let barrier = Arc::new(Barrier::new(worker_count));
    let mut handles = Vec::new();

    for index in 0..worker_count {
        let barrier = Arc::clone(&barrier);
        let database_url = database_url.clone();

        handles.push(thread::spawn(
            move || -> Result<Option<(Uuid, Uuid, String)>, String> {
                let mut runtime =
                    Runtime::connect_postgres(&database_url).map_err(|error| error.to_string())?;
                barrier.wait();
                let leased = runtime
                    .lease_next_task(LeaseNextTaskRequest {
                        worker_id: format!("worker-{index}"),
                        lease_duration_ms: 30_000,
                    })
                    .map_err(|error| error.to_string())?;

                Ok(leased.map(|task| {
                    let lease = task.lease.expect("leased projection should include lease");
                    (task.task_id, lease.lease_id, lease.worker_id)
                }))
            },
        ));
    }

    let mut winners = Vec::new();
    for handle in handles {
        let result = handle
            .join()
            .map_err(|_| std::io::Error::other("lease thread panicked"))?
            .map_err(std::io::Error::other)?;
        if let Some(winner) = result {
            winners.push(winner);
        }
    }

    assert_eq!(winners.len(), 1);
    assert_eq!(winners[0].0, task_id);
    let shown = runtime.show_task(started.session_id, task_id)?;
    assert_eq!(shown.status, TASK_LEASED_STATE);
    assert_eq!(
        shown.lease.as_ref().expect("lease slot").lease_id,
        winners[0].1
    );
    assert_eq!(
        shown.lease.as_ref().expect("lease slot").worker_id,
        winners[0].2
    );
    assert!(
        runtime
            .lease_next_task(LeaseNextTaskRequest::new("late-worker"))?
            .is_none()
    );

    Ok(())
}

#[test]
fn task_lease_heartbeat_renews_active_deadline() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("task-heartbeat-fixture"))?;
    let task_id = create_and_enqueue_task_with_max_retries(
        &mut runtime,
        started.session_id,
        "write the heartbeat task",
        1,
    )?;
    let leased = runtime
        .lease_next_task(LeaseNextTaskRequest {
            worker_id: "worker-heartbeat".to_owned(),
            lease_duration_ms: 1,
        })?
        .expect("task should be leased");
    let lease = leased.lease.as_ref().expect("lease slot");
    let old_deadline = lease
        .lease_deadline_ms
        .expect("initial lease deadline should be recorded");

    let renewed = runtime.heartbeat_task_lease(
        task_id,
        lease.lease_id,
        HeartbeatTaskLeaseRequest {
            worker_id: "worker-heartbeat".to_owned(),
            lease_duration_ms: 60_000,
        },
    )?;
    let renewed_lease = renewed.lease.as_ref().expect("renewed lease slot");
    let renewed_deadline = renewed_lease
        .lease_deadline_ms
        .expect("renewed lease deadline should be recorded");
    let events = runtime.events_for_session(started.session_id)?;

    assert_eq!(renewed.status, TASK_LEASED_STATE);
    assert_eq!(renewed_lease.worker_id, "worker-heartbeat");
    assert!(renewed_deadline > old_deadline);
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::TaskLeaseRenewed)
    );

    Ok(())
}

#[test]
fn expired_task_lease_requeues_for_retry_and_reassigns() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("task-retry-fixture"))?;
    let task_id = create_and_enqueue_task_with_max_retries(
        &mut runtime,
        started.session_id,
        "write the retryable task",
        1,
    )?;
    runtime
        .lease_next_task(LeaseNextTaskRequest::new("worker-expiring"))?
        .expect("task should be leased");

    let expired = runtime.expire_task_leases(i64::MAX)?;

    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].task_id, task_id);
    assert_eq!(expired[0].status, TASK_RETRY_QUEUED_STATE);
    let queue = expired[0].queue.as_ref().expect("retry queue slot");
    assert_eq!(queue.status, TASK_RETRY_QUEUED_STATE);
    assert_eq!(queue.retry_count, Some(1));
    assert_eq!(queue.max_retries, Some(1));
    assert_eq!(
        queue.stop_reason.as_deref(),
        Some(TASK_STOP_REASON_LEASE_EXPIRED)
    );
    let lease = expired[0].lease.as_ref().expect("expired lease slot");
    assert_eq!(lease.status, "expired");

    let reassigned = runtime
        .lease_next_task(LeaseNextTaskRequest::new("worker-retry"))?
        .expect("retry queued task should be reassigned");
    let reassigned_queue = reassigned.queue.as_ref().expect("reassigned queue slot");
    let reassigned_lease = reassigned.lease.as_ref().expect("reassigned lease slot");
    assert_eq!(reassigned.task_id, task_id);
    assert_eq!(reassigned.status, TASK_LEASED_STATE);
    assert_eq!(reassigned_queue.retry_count, Some(1));
    assert_eq!(reassigned_lease.worker_id, "worker-retry");

    Ok(())
}

#[test]
fn expired_task_lease_stops_at_retry_limit() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("task-stop-fixture"))?;
    let task_id = create_and_enqueue_task_with_max_retries(
        &mut runtime,
        started.session_id,
        "write the stopped task",
        0,
    )?;
    runtime
        .lease_next_task(LeaseNextTaskRequest::new("worker-stop"))?
        .expect("task should be leased");

    let expired = runtime.expire_task_leases(i64::MAX)?;

    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].task_id, task_id);
    assert_eq!(expired[0].status, TASK_STOPPED_STATE);
    let queue = expired[0].queue.as_ref().expect("stopped queue slot");
    assert_eq!(queue.status, TASK_STOPPED_STATE);
    assert_eq!(queue.retry_count, Some(0));
    assert_eq!(queue.max_retries, Some(0));
    assert_eq!(
        queue.stop_reason.as_deref(),
        Some(TASK_STOP_REASON_MAX_RETRIES_EXCEEDED)
    );
    let lease = expired[0].lease.as_ref().expect("stopped lease slot");
    assert_eq!(lease.status, TASK_STOPPED_STATE);
    assert!(
        runtime
            .lease_next_task(LeaseNextTaskRequest::new("late-worker"))?
            .is_none()
    );

    Ok(())
}

#[test]
fn task_projection_replays_task_scoped_status_slots() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("task-slot-fixture"))?;
    let task = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("write the task scoped fake patch"),
    )?;
    let store = PostgresEventStore::connect(&database_url)?;

    store.append_event(NewEvent::worker_lane_worktree_allocated(
        started.session_id,
        WorkerLaneWorktreeAllocatedPayload::new(
            "lane-1",
            DEFAULT_TASK_WORKER_LANE_KIND,
            "C:/repo",
            "C:/repo-worktree",
            "HEAD",
        )
        .with_task_id(task.task_id),
    )?)?;
    store.append_event(NewEvent::worker_lane_observation_recorded(
        started.session_id,
        WorkerLaneObservationPayload {
            task_id: Some(task.task_id),
            lane_id: "lane-1".to_owned(),
            lane_kind: DEFAULT_TASK_WORKER_LANE_KIND.to_owned(),
            status: "succeeded".to_owned(),
            exit_code: Some(0),
            stdout: "worker output".to_owned(),
            stderr: String::new(),
            duration_ms: 42,
            prompt_tokens: Some(100),
            completion_tokens: Some(25),
            usage_confidence: "local_estimate".to_owned(),
        },
    )?)?;
    store.append_event(NewEvent::diff_recorded(
        started.session_id,
        DiffSummaryPayload::new(1, 3, 0, vec![".harness/fake-agent-turn.md".to_owned()])
            .with_task_id(task.task_id),
    )?)?;
    store.append_event(NewEvent::commit_approval_pending(
        started.session_id,
        CommitApprovalPendingPayload::new(
            PENDING_COMMIT_APPROVAL_STATE,
            "verification passed; awaiting human commit approval",
        )
        .with_task_id(task.task_id),
    )?)?;
    store.append_event(NewEvent::commit_approved(
        started.session_id,
        CommitApprovalDecisionPayload::new(
            COMMIT_APPROVED_STATE,
            "approved by runtime approval state machine",
            "runtime",
        )
        .with_task_id(task.task_id),
    )?)?;
    store.append_event(NewEvent::commit_succeeded(
        started.session_id,
        CommitHandoffPayload::succeeded(
            "C:/repo-worktree",
            "Commit approved task patch",
            "runtime",
            "0123456789abcdef0123456789abcdef01234567",
        )
        .with_task_id(task.task_id),
    )?)?;

    let shown = runtime.show_task(started.session_id, task.task_id)?;

    assert_eq!(shown.status, COMMITTED_STATE);
    let worktree = shown.worktree.expect("task worktree slot");
    assert_eq!(worktree.lane_id, "lane-1");
    assert_eq!(worktree.worktree_path, "C:/repo-worktree");
    let worker_output = shown.worker_output.expect("task worker output slot");
    assert_eq!(worker_output.status, "succeeded");
    assert_eq!(worker_output.stdout, "worker output");
    assert_eq!(worker_output.exit_code, Some(0));
    let diff = shown.diff.expect("task diff slot");
    assert_eq!(diff.files_changed, 1);
    assert_eq!(diff.paths, vec![".harness/fake-agent-turn.md"]);
    let approval = shown.approval.expect("task approval slot");
    assert_eq!(approval.state, COMMIT_APPROVED_STATE);
    assert_eq!(approval.rejection_reason, None);
    let commit = shown.commit.expect("task commit slot");
    assert_eq!(commit.state, COMMITTED_STATE);
    assert_eq!(
        commit.commit_sha.as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(shown.event_count, 8);

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
fn task_queue_row_is_not_leaseable_before_enqueued_event_commit()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let store = PostgresEventStore::connect(&database_url)?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("atomic-enqueue-fixture"))?;
    let task = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("queue atomically"),
    )?;

    store.enqueue_task(started.session_id, task.task_id, 1, |record| {
        let mut worker_runtime = Runtime::connect_postgres(&database_url)?;
        let leased = worker_runtime
            .lease_next_task(LeaseNextTaskRequest::new("worker-before-enqueue-commit"))?;
        assert!(leased.is_none());

        NewEvent::task_enqueued(
            record.session_id,
            TaskQueuePayload::new(
                record.task_id,
                TASK_QUEUED_STATE,
                record
                    .last_reason
                    .as_deref()
                    .unwrap_or("task enqueued for worker execution"),
            )
            .with_retry_state(
                record.retry_count,
                record.max_retries,
                record.stop_reason.clone(),
            ),
        )
    })?;

    let leased = runtime
        .lease_next_task(LeaseNextTaskRequest::new("worker-after-enqueue-commit"))?
        .expect("task should be leaseable after enqueue event commits");
    let queue_record = store
        .task_queue_record(task.task_id)?
        .expect("queue row should exist");

    assert_eq!(leased.status, TASK_LEASED_STATE);
    assert_eq!(queue_record.status, TASK_LEASED_STATE);

    Ok(())
}

#[test]
fn task_queue_event_failures_roll_back_visible_queue_mutations()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let store = PostgresEventStore::connect(&database_url)?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("queue-rollback-fixture"))?;
    let task = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("rollback enqueue event failure"),
    )?;

    let error = store
        .enqueue_task(started.session_id, task.task_id, 1, |_record| {
            Err(HarnessError::new("injected enqueue event failure"))
        })
        .expect_err("injected event failure should abort enqueue transaction");
    assert!(error.to_string().contains("injected enqueue event failure"));
    assert!(store.task_queue_record(task.task_id)?.is_none());
    assert!(
        runtime
            .lease_next_task(LeaseNextTaskRequest::new("worker-after-failed-enqueue"))?
            .is_none()
    );

    runtime.enqueue_task(started.session_id, task.task_id)?;
    let lease_error = store
        .lease_next_queued_task(
            "worker-failed-lease-event",
            Uuid::new_v4(),
            current_time_ms_for_test()? + 60_000,
            |_record| Err(HarnessError::new("injected lease event failure")),
        )
        .expect_err("injected event failure should abort lease transaction");
    assert!(
        lease_error
            .to_string()
            .contains("injected lease event failure")
    );

    let queue_record = store
        .task_queue_record(task.task_id)?
        .expect("queue row should remain after successful enqueue");
    assert_eq!(queue_record.status, TASK_QUEUED_STATE);

    let leased = runtime
        .lease_next_task(LeaseNextTaskRequest::new("worker-after-failed-lease"))?
        .expect("task should remain leaseable after failed lease transaction");
    let queue_record = store
        .task_queue_record(task.task_id)?
        .expect("queue row should exist after lease");
    assert_eq!(leased.status, TASK_LEASED_STATE);
    assert_eq!(queue_record.status, TASK_LEASED_STATE);

    Ok(())
}

#[test]
fn terminal_and_expire_event_failures_roll_back_queue_mutations()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let store = PostgresEventStore::connect(&database_url)?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("queue-terminal-rollback"))?;

    let terminal_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "rollback terminal event failure",
    )?;
    let terminal_lease = runtime
        .lease_next_task(LeaseNextTaskRequest::new("terminal-worker"))?
        .expect("terminal task should lease");
    let terminal_lease_id = terminal_lease.lease.as_ref().expect("lease slot").lease_id;

    let terminal_error = store
        .transition_leased_task(
            terminal_task_id,
            terminal_lease_id,
            TASK_COMPLETED_STATE,
            "complete with injected event failure",
            current_time_ms_for_test()?,
            |_record| Err(HarnessError::new("injected terminal event failure")),
        )
        .expect_err("injected event failure should abort terminal transaction");
    assert!(
        terminal_error
            .to_string()
            .contains("injected terminal event failure")
    );
    let queue_record = store
        .task_queue_record(terminal_task_id)?
        .expect("terminal task queue row should exist");
    assert_eq!(queue_record.status, TASK_LEASED_STATE);
    assert_eq!(
        runtime
            .show_task(started.session_id, terminal_task_id)?
            .status,
        TASK_LEASED_STATE
    );

    let expire_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "rollback expire event failure",
    )?;
    let mut expire_request = LeaseNextTaskRequest::new("expire-worker");
    expire_request.lease_duration_ms = 1;
    runtime
        .lease_next_task(expire_request)?
        .expect("expire task should lease");
    let expire_error = store
        .expire_due_task_leases(current_time_ms_for_test()? + 60_000, |_record| {
            Err(HarnessError::new("injected expire event failure"))
        })
        .expect_err("injected event failure should abort expire transaction");
    assert!(
        expire_error
            .to_string()
            .contains("injected expire event failure")
    );

    let queue_record = store
        .task_queue_record(expire_task_id)?
        .expect("expire task queue row should exist");
    assert_eq!(queue_record.status, TASK_LEASED_STATE);
    assert_eq!(
        runtime
            .show_task(started.session_id, expire_task_id)?
            .status,
        TASK_LEASED_STATE
    );

    Ok(())
}

#[test]
fn task_queue_successful_transitions_keep_eventlog_replay_and_queue_status_aligned()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let store = PostgresEventStore::connect(&database_url)?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started =
        runtime.start_session(StartSessionRequest::new("queue-eventlog-success-alignment"))?;

    let assert_replay_and_queue_status = |runtime: &Runtime,
                                          task_id: Uuid,
                                          expected_status: &str|
     -> Result<(), Box<dyn std::error::Error>> {
        let projected = runtime.show_task(started.session_id, task_id)?;
        let queue_record = store
            .task_queue_record(task_id)?
            .expect("task queue row should exist");

        assert_eq!(projected.status, expected_status);
        assert_eq!(queue_record.status, expected_status);

        Ok(())
    };

    let complete_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "complete atomically with event evidence",
    )?;
    let complete_lease = runtime
        .lease_next_task(LeaseNextTaskRequest::new("complete-atomic-worker"))?
        .expect("complete task should lease");
    let complete_lease_id = complete_lease.lease.as_ref().expect("lease slot").lease_id;
    runtime.complete_task_lease(
        complete_task_id,
        complete_lease_id,
        "complete with durable event evidence",
    )?;
    assert_replay_and_queue_status(&runtime, complete_task_id, TASK_COMPLETED_STATE)?;

    let fail_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "fail atomically with event evidence",
    )?;
    let fail_lease = runtime
        .lease_next_task(LeaseNextTaskRequest::new("fail-atomic-worker"))?
        .expect("fail task should lease");
    let fail_lease_id = fail_lease.lease.as_ref().expect("lease slot").lease_id;
    runtime.fail_task_lease(
        fail_task_id,
        fail_lease_id,
        "fail with durable event evidence",
    )?;
    assert_replay_and_queue_status(&runtime, fail_task_id, TASK_FAILED_STATE)?;

    let cancel_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "cancel atomically with event evidence",
    )?;
    let cancel_lease = runtime
        .lease_next_task(LeaseNextTaskRequest::new("cancel-atomic-worker"))?
        .expect("cancel task should lease");
    let cancel_lease_id = cancel_lease.lease.as_ref().expect("lease slot").lease_id;
    runtime.cancel_task_lease(
        cancel_task_id,
        cancel_lease_id,
        "cancel with durable event evidence",
    )?;
    assert_replay_and_queue_status(&runtime, cancel_task_id, TASK_CANCELLED_STATE)?;

    let retry_task = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("expire atomically into retry"),
    )?;
    runtime.enqueue_task_with_max_retries(started.session_id, retry_task.task_id, 1)?;
    let mut retry_lease_request = LeaseNextTaskRequest::new("retry-expire-worker");
    retry_lease_request.lease_duration_ms = 1;
    let retry_lease = runtime
        .lease_next_task(retry_lease_request)?
        .expect("retry expiration task should lease");
    assert_eq!(retry_lease.task_id, retry_task.task_id);
    runtime.expire_task_leases(current_time_ms_for_test()?.saturating_add(60_000))?;
    assert_replay_and_queue_status(&runtime, retry_task.task_id, TASK_RETRY_QUEUED_STATE)?;
    let retry_cleanup_lease = runtime
        .lease_next_task(LeaseNextTaskRequest::new("retry-cleanup-worker"))?
        .expect("retry task should lease for cleanup before stopped path");
    assert_eq!(retry_cleanup_lease.task_id, retry_task.task_id);
    let retry_cleanup_lease_id = retry_cleanup_lease
        .lease
        .as_ref()
        .expect("retry cleanup lease slot")
        .lease_id;
    runtime.complete_task_lease(
        retry_task.task_id,
        retry_cleanup_lease_id,
        "cleanup retry task before stopped expiration path",
    )?;

    let stopped_task = runtime.create_task(
        started.session_id,
        CreateTaskRequest::new("expire atomically into stopped"),
    )?;
    runtime.enqueue_task_with_max_retries(started.session_id, stopped_task.task_id, 0)?;
    let mut stopped_lease_request = LeaseNextTaskRequest::new("stopped-expire-worker");
    stopped_lease_request.lease_duration_ms = 1;
    let stopped_lease = runtime
        .lease_next_task(stopped_lease_request)?
        .expect("stopped expiration task should lease");
    assert_eq!(stopped_lease.task_id, stopped_task.task_id);
    runtime.expire_task_leases(current_time_ms_for_test()?.saturating_add(60_000))?;
    assert_replay_and_queue_status(&runtime, stopped_task.task_id, TASK_STOPPED_STATE)?;

    let events = runtime.events_for_session(started.session_id)?;
    let complete_task_id_text = complete_task_id.to_string();
    let task_events = events
        .iter()
        .filter(|event| {
            event
                .payload
                .get("task_id")
                .and_then(|value| value.as_str())
                .is_some_and(|value| value == complete_task_id_text.as_str())
        })
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert_eq!(
        task_events,
        vec![
            EventType::TaskCreated,
            EventType::TaskEnqueued,
            EventType::TaskLeaseAcquired,
            EventType::TaskLeaseCompleted,
        ]
    );

    Ok(())
}

#[test]
fn expired_lease_owner_cannot_terminal_transition_task() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let store = PostgresEventStore::connect(&database_url)?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new("expired-terminal-reject"))?;

    let mut lease_expired_task = |input: &str| -> Result<(Uuid, Uuid), Box<dyn std::error::Error>> {
        let task_id = create_and_enqueue_task(&mut runtime, started.session_id, input)?;
        let lease_id = Uuid::new_v4();
        let deadline_ms = current_time_ms_for_test()?.saturating_sub(1);
        let record = store
            .lease_next_queued_task("expired-worker", lease_id, deadline_ms, |record| {
                NewEvent::task_lease_acquired(
                    record.session_id,
                    TaskLeasePayload::new(
                        record.task_id,
                        record
                            .lease_id
                            .ok_or_else(|| HarnessError::new("lease id missing"))?,
                        record
                            .worker_id
                            .as_deref()
                            .ok_or_else(|| HarnessError::new("worker id missing"))?,
                        record.status.as_str(),
                        record.lease_deadline_ms,
                        record
                            .last_reason
                            .as_deref()
                            .unwrap_or("task lease acquired"),
                    )
                    .with_retry_state(
                        record.retry_count,
                        record.max_retries,
                        record.stop_reason.clone(),
                    ),
                )
            })?
            .expect("task should lease with expired deadline");
        assert_eq!(record.task_id, task_id);

        Ok((task_id, lease_id))
    };

    let (complete_task_id, complete_lease_id) = lease_expired_task("expired complete should fail")?;
    let (fail_task_id, fail_lease_id) = lease_expired_task("expired fail should fail")?;
    let (cancel_task_id, cancel_lease_id) = lease_expired_task("expired cancel should fail")?;
    let wrong_lease_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "wrong lease id should fail distinctly",
    )?;
    let mut wrong_lease_request = LeaseNextTaskRequest::new("wrong-lease-worker");
    wrong_lease_request.lease_duration_ms = 600_000;
    let wrong_lease = runtime
        .lease_next_task(wrong_lease_request)?
        .expect("wrong lease id task should lease");
    assert_eq!(wrong_lease.task_id, wrong_lease_task_id);

    let complete_error = runtime
        .complete_task_lease(
            complete_task_id,
            complete_lease_id,
            "late complete should not win",
        )
        .expect_err("expired complete should be rejected");
    assert!(complete_error.to_string().contains("task lease expired"));

    let wrong_lease_error = runtime
        .complete_task_lease(
            wrong_lease_task_id,
            Uuid::new_v4(),
            "wrong lease id should not look expired",
        )
        .expect_err("wrong lease id should be rejected distinctly");
    assert!(
        wrong_lease_error
            .to_string()
            .contains("active task lease was not found")
    );
    assert_ne!(wrong_lease_error.to_string(), complete_error.to_string());

    let fail_error = runtime
        .fail_task_lease(fail_task_id, fail_lease_id, "late fail should not win")
        .expect_err("expired fail should be rejected");
    assert!(fail_error.to_string().contains("task lease expired"));

    let cancel_error = runtime
        .cancel_task_lease(
            cancel_task_id,
            cancel_lease_id,
            "late cancel should not win",
        )
        .expect_err("expired cancel should be rejected");
    assert!(cancel_error.to_string().contains("task lease expired"));

    let events = runtime.events_for_session(started.session_id)?;
    assert!(
        !events
            .iter()
            .any(|event| event.event_type == EventType::TaskLeaseCompleted)
    );
    assert!(
        !events
            .iter()
            .any(|event| event.event_type == EventType::TaskLeaseFailed)
    );
    assert!(
        !events
            .iter()
            .any(|event| event.event_type == EventType::TaskLeaseCancelled)
    );

    assert_eq!(
        runtime
            .show_task(started.session_id, complete_task_id)?
            .status,
        TASK_LEASED_STATE
    );
    assert_eq!(
        runtime.show_task(started.session_id, fail_task_id)?.status,
        TASK_LEASED_STATE
    );
    assert_eq!(
        runtime
            .show_task(started.session_id, cancel_task_id)?
            .status,
        TASK_LEASED_STATE
    );

    let expired = runtime.expire_task_leases(current_time_ms_for_test()? + 60_000)?;
    assert_eq!(expired.len(), 3);
    assert!(
        expired
            .iter()
            .all(|task| task.status == TASK_RETRY_QUEUED_STATE)
    );

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
fn leased_task_runs_codex_worker_lane_and_enters_pending_approval()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let (script_name, script) = fake_success_runner_script();
    let repo = git_fixture_repo_with_files(&[(script_name, script)])?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;
    let first_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write the first queued worker task",
    )?;
    let second_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write the second queued worker task",
    )?;
    let mut worker = CodexWorkerLaneRequest::new_subprocess(
        "placeholder task should be replaced",
        fake_runner_command(script_name).with_env("HARNESS_CUSTOM_MARKER", "leased-task"),
    );
    worker.budget.max_stdout_bytes = 4;

    let result = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "queue-worker-1",
            worker,
        ))?
        .expect("queued task should be leased and run");
    let events = runtime.events_for_session(started.session_id)?;
    let diff_event = events
        .iter()
        .find(|event| event.event_type == EventType::DiffRecorded)
        .expect("diff event");
    let pending_event = events
        .iter()
        .find(|event| event.event_type == EventType::CommitApprovalPending)
        .expect("pending approval event");
    let shown_first = runtime.show_task(started.session_id, first_task_id)?;
    let shown_second = runtime.show_task(started.session_id, second_task_id)?;

    assert_eq!(result.task.task_id, first_task_id);
    assert_eq!(result.worker.final_status, WorkerLaneStatus::Succeeded);
    assert_eq!(
        result.worker.pending_commit_state.as_deref(),
        Some(PENDING_COMMIT_APPROVAL_STATE)
    );
    assert_eq!(result.task.status, PENDING_COMMIT_APPROVAL_STATE);
    assert_eq!(
        result.task.lease.as_ref().expect("lease slot").status,
        TASK_COMPLETED_STATE
    );
    assert_eq!(
        result.task.queue.as_ref().expect("queue slot").status,
        TASK_COMPLETED_STATE
    );
    assert_eq!(
        result
            .task
            .worker_output
            .as_ref()
            .expect("worker output")
            .status,
        "succeeded"
    );
    assert_eq!(
        result.task.diff.as_ref().expect("diff slot").paths,
        vec!["README.md"]
    );
    assert_eq!(
        result.task.approval.as_ref().expect("approval slot").state,
        PENDING_COMMIT_APPROVAL_STATE
    );
    assert_eq!(shown_first.status, PENDING_COMMIT_APPROVAL_STATE);
    assert_eq!(shown_second.status, TASK_QUEUED_STATE);
    assert_eq!(diff_event.payload["task_id"], first_task_id.to_string());
    assert_eq!(pending_event.payload["task_id"], first_task_id.to_string());
    assert!(events.iter().any(|event| {
        event.event_type == EventType::WorkerLaneObservationRecorded
            && event.payload["task_id"] == first_task_id.to_string()
            && event.payload["status"] == "succeeded"
    }));

    Ok(())
}

#[test]
fn leased_codex_worker_uses_task_repo_lane_and_budget() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let (script_name, script) = fake_success_runner_script();
    let session_repo = git_fixture_repo()?;
    let task_repo = git_fixture_repo_with_files(&[(script_name, script)])?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started =
        runtime.start_session(StartSessionRequest::new(session_repo.display().to_string()))?;

    let mut other_lane_request = CreateTaskRequest::new("run with a future worker lane");
    other_lane_request.worker_lane_kind = "future_worker_lane".to_owned();
    let other_lane_task = runtime.create_task(started.session_id, other_lane_request)?;
    runtime.enqueue_task(started.session_id, other_lane_task.task_id)?;

    let mut codex_request = CreateTaskRequest::new("write the override repo queued task");
    codex_request.repo_path = Some(task_repo.display().to_string());
    codex_request.context.budget.max_bytes = 1_234;
    codex_request.max_output_tokens = 17;
    let codex_task = runtime.create_task(started.session_id, codex_request)?;
    runtime.enqueue_task(started.session_id, codex_task.task_id)?;

    let mut worker = CodexWorkerLaneRequest::new_subprocess(
        "placeholder task should be replaced",
        fake_runner_command(script_name),
    );
    worker.budget.max_prompt_tokens = 99_999;
    worker.budget.max_output_tokens = 88_888;
    worker.budget.max_stdout_bytes = 4;

    let result = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "task-source-worker",
            worker,
        ))?
        .expect("codex task should be leased");
    let events = runtime.events_for_session(started.session_id)?;
    let requested = events
        .iter()
        .find(|event| {
            event.event_type == EventType::WorkerLaneRequested
                && event.payload["task_id"] == codex_task.task_id.to_string()
        })
        .expect("task-scoped worker request");
    let allocated = result.task.worktree.as_ref().expect("task worktree");
    let shown_other = runtime.show_task(started.session_id, other_lane_task.task_id)?;

    assert_eq!(result.task.task_id, codex_task.task_id);
    assert_eq!(shown_other.status, TASK_QUEUED_STATE);
    assert_eq!(shown_other.worker_lane_kind, "future_worker_lane");
    assert_eq!(result.task.worker_lane_kind, DEFAULT_TASK_WORKER_LANE_KIND);
    assert_eq!(requested.payload["budget"]["max_prompt_tokens"], 1_234);
    assert_eq!(requested.payload["budget"]["max_output_tokens"], 17);
    assert_eq!(requested.payload["budget"]["max_stdout_bytes"], 4);
    assert!(allocated.worktree_path.contains(".harness-worktrees"));
    assert!(
        allocated.worktree_path.contains(
            task_repo
                .file_name()
                .and_then(|name| name.to_str())
                .expect("task repo name")
        )
    );
    assert_eq!(
        git_text(&session_repo, &["status", "--short"])?,
        "",
        "session repo should not receive task repo worker changes"
    );
    assert!(
        result
            .task
            .diff
            .as_ref()
            .expect("task diff")
            .paths
            .contains(&"README.md".to_owned())
    );

    Ok(())
}

#[test]
fn task_scoped_approval_and_commit_handoff_replay() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let (script_name, script) = fake_success_runner_script();
    let repo = git_fixture_repo_with_files(&[(script_name, script)])?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;
    let task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write the task-scoped approval commit task",
    )?;
    let result = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "task-approval-worker",
            CodexWorkerLaneRequest::new_subprocess("placeholder", fake_runner_command(script_name)),
        ))?
        .expect("queued task should run");
    let worktree = result
        .task
        .worktree
        .as_ref()
        .expect("task worktree")
        .worktree_path
        .clone();
    let worktree = PathBuf::from(worktree);
    let commit_count_before = git_commit_count(&worktree)?;

    let session_approval_error = runtime
        .approve_pending_diff(started.session_id)
        .expect_err("session approval should ignore task-scoped pending approvals");
    assert!(
        session_approval_error
            .to_string()
            .contains("pending diff was not recorded")
    );
    assert_eq!(
        runtime.show_task(started.session_id, task_id)?.status,
        PENDING_COMMIT_APPROVAL_STATE
    );

    let approved = runtime.approve_task_pending_diff(started.session_id, task_id)?;
    assert_eq!(approved.status, COMMIT_APPROVED_STATE);
    assert_eq!(
        approved.approval.as_ref().expect("approval slot").state,
        COMMIT_APPROVED_STATE
    );

    let committed = runtime.commit_approved_task_diff(
        started.session_id,
        task_id,
        "Commit queued task diff",
    )?;
    let events = runtime.events_for_session(started.session_id)?;

    assert_eq!(committed.status, COMMITTED_STATE);
    assert_eq!(
        committed.commit.as_ref().expect("commit slot").state,
        COMMITTED_STATE
    );
    assert!(
        committed
            .commit
            .as_ref()
            .expect("commit slot")
            .commit_sha
            .as_deref()
            .is_some_and(|sha| sha.len() == 40)
    );
    assert_eq!(git_commit_count(&worktree)?, commit_count_before + 1);
    assert_eq!(git_text(&worktree, &["status", "--short"])?, "");
    assert!(events.iter().any(|event| {
        event.event_type == EventType::CommitApproved
            && event.payload["task_id"] == task_id.to_string()
    }));
    assert!(events.iter().any(|event| {
        event.event_type == EventType::CommitStarted
            && event.payload["task_id"] == task_id.to_string()
    }));
    assert!(events.iter().any(|event| {
        event.event_type == EventType::CommitSucceeded
            && event.payload["task_id"] == task_id.to_string()
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            event.event_type,
            EventType::CommitApproved | EventType::CommitStarted | EventType::CommitSucceeded
        ) && event.payload.get("task_id").is_none()
    }));

    Ok(())
}

#[test]
fn task_scoped_rejection_and_commit_failure_replay() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let (script_name, script) = fake_success_runner_script();
    let reject_repo = git_fixture_repo_with_files(&[(script_name, script)])?;
    let fail_repo = git_fixture_repo_with_files(&[(script_name, script)])?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let reject_session =
        runtime.start_session(StartSessionRequest::new(reject_repo.display().to_string()))?;
    let reject_task_id = create_and_enqueue_task(
        &mut runtime,
        reject_session.session_id,
        "write the rejected task-scoped approval task",
    )?;
    runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "task-reject-worker",
            CodexWorkerLaneRequest::new_subprocess("placeholder", fake_runner_command(script_name)),
        ))?
        .expect("reject task should run");

    let rejected = runtime.reject_task_pending_diff(
        reject_session.session_id,
        reject_task_id,
        "not safe for commit",
    )?;
    let rejected_commit = runtime
        .commit_approved_task_diff(
            reject_session.session_id,
            reject_task_id,
            "Should not commit",
        )
        .expect_err("rejected task should not commit");
    assert_eq!(rejected.status, COMMIT_REJECTED_STATE);
    assert_eq!(
        rejected
            .approval
            .as_ref()
            .expect("approval slot")
            .rejection_reason
            .as_deref(),
        Some("not safe for commit")
    );
    assert!(
        rejected_commit
            .to_string()
            .contains("current state is rejected")
    );

    let fail_session =
        runtime.start_session(StartSessionRequest::new(fail_repo.display().to_string()))?;
    let fail_task_id = create_and_enqueue_task(
        &mut runtime,
        fail_session.session_id,
        "write the failed task-scoped commit task",
    )?;
    let failed_run = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "task-fail-worker",
            CodexWorkerLaneRequest::new_subprocess("placeholder", fake_runner_command(script_name)),
        ))?
        .expect("failed commit task should run");
    let fail_worktree = PathBuf::from(
        failed_run
            .task
            .worktree
            .as_ref()
            .expect("task worktree")
            .worktree_path
            .clone(),
    );
    run_git(&fail_worktree, &["checkout", "--", "README.md"])?;
    runtime.approve_task_pending_diff(fail_session.session_id, fail_task_id)?;

    let failed =
        runtime.commit_approved_task_diff(fail_session.session_id, fail_task_id, "Commit fails")?;
    let events = runtime.events_for_session(fail_session.session_id)?;

    assert_eq!(failed.status, COMMIT_FAILED_STATE);
    assert_eq!(
        failed.commit.as_ref().expect("commit slot").state,
        COMMIT_FAILED_STATE
    );
    assert!(
        failed
            .commit
            .as_ref()
            .expect("commit slot")
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("approved diff has no local changes"))
    );
    assert!(events.iter().any(|event| {
        event.event_type == EventType::CommitFailed
            && event.payload["task_id"] == fail_task_id.to_string()
    }));

    Ok(())
}

#[test]
fn queued_worker_lease_deadline_covers_worker_timeout() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = git_fixture_repo()?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;
    create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write the long timeout leased task",
    )?;
    let mut worker = CodexWorkerLaneRequest::new(
        "placeholder",
        CodexWorkerLaneFixture::succeeded("long timeout worker"),
    );
    worker.timeout_ms = 120_000;
    let before_ms = current_time_ms_for_test()?;

    let result = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "queue-worker-long-timeout",
            worker,
        ))?
        .expect("long timeout task should run");
    let lease_deadline_ms = result
        .task
        .lease
        .as_ref()
        .expect("lease slot")
        .lease_deadline_ms
        .expect("lease deadline");

    assert!(
        lease_deadline_ms >= before_ms + 121_000,
        "lease deadline {lease_deadline_ms} should cover timeout plus margin from {before_ms}"
    );
    assert!(runtime.expire_task_leases(before_ms + 60_500)?.is_empty());

    Ok(())
}

#[test]
fn invalid_queued_worker_config_is_rejected_before_lease() -> Result<(), Box<dyn std::error::Error>>
{
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = git_fixture_repo()?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;

    let timeout_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "keep queued when timeout is invalid",
    )?;
    let mut timeout_worker = CodexWorkerLaneRequest::new(
        "placeholder",
        CodexWorkerLaneFixture::succeeded("invalid timeout worker"),
    );
    timeout_worker.timeout_ms = 0;

    let timeout_error = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "invalid-timeout-worker",
            timeout_worker,
        ))
        .expect_err("invalid timeout should fail before leasing");
    assert!(timeout_error.to_string().contains("timeout"));

    let timeout_task = runtime.show_task(started.session_id, timeout_task_id)?;
    assert_eq!(timeout_task.status, TASK_QUEUED_STATE);
    assert_eq!(
        timeout_task.queue.as_ref().expect("queue slot").status,
        TASK_QUEUED_STATE
    );
    assert!(timeout_task.lease.is_none());

    let stdout_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "keep queued when stdout budget is invalid",
    )?;
    let mut stdout_worker = CodexWorkerLaneRequest::new(
        "placeholder",
        CodexWorkerLaneFixture::succeeded("invalid stdout worker"),
    );
    stdout_worker.budget.max_stdout_bytes = 0;

    let stdout_error = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "invalid-stdout-worker",
            stdout_worker,
        ))
        .expect_err("invalid stdout budget should fail before leasing");
    assert!(stdout_error.to_string().contains("stdout budget"));

    let stdout_task = runtime.show_task(started.session_id, stdout_task_id)?;
    assert_eq!(stdout_task.status, TASK_QUEUED_STATE);
    assert_eq!(
        stdout_task.queue.as_ref().expect("queue slot").status,
        TASK_QUEUED_STATE
    );
    assert!(stdout_task.lease.is_none());

    Ok(())
}

#[test]
fn leased_task_worker_failure_timeout_and_cancellation_update_task_state()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = git_fixture_repo()?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;
    let failed_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write the failed leased task",
    )?;
    let timed_out_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write the timed out leased task",
    )?;
    let cancelled_task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write the cancelled leased task",
    )?;

    let failed = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "queue-worker-fail",
            CodexWorkerLaneRequest::new(
                "placeholder",
                CodexWorkerLaneFixture {
                    status: WorkerLaneStatus::Failed,
                    exit_code: Some(7),
                    stdout: "failure stdout".to_owned(),
                    stderr: "failure stderr".to_owned(),
                    duration_ms: 7,
                    usage: CodexWorkerUsage::unknown(),
                },
            ),
        ))?
        .expect("failed task should run");

    let mut timeout_worker = CodexWorkerLaneRequest::new(
        "placeholder",
        CodexWorkerLaneFixture {
            status: WorkerLaneStatus::Succeeded,
            exit_code: Some(0),
            stdout: "timeout stdout".to_owned(),
            stderr: String::new(),
            duration_ms: 100,
            usage: CodexWorkerUsage::unknown(),
        },
    );
    timeout_worker.timeout_ms = 1;
    let timed_out = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "queue-worker-timeout",
            timeout_worker,
        ))?
        .expect("timed out task should run");

    let mut cancelled_worker = CodexWorkerLaneRequest::new(
        "placeholder",
        CodexWorkerLaneFixture::succeeded("should not run"),
    );
    cancelled_worker.cancellation_requested = true;
    let cancelled = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "queue-worker-cancel",
            cancelled_worker,
        ))?
        .expect("cancelled task should run");

    assert_eq!(failed.task.task_id, failed_task_id);
    assert_eq!(failed.worker.final_status, WorkerLaneStatus::Failed);
    assert_eq!(failed.task.status, TASK_FAILED_STATE);
    assert_eq!(
        failed.task.lease.as_ref().expect("lease slot").status,
        TASK_FAILED_STATE
    );
    assert_eq!(
        failed
            .task
            .worker_output
            .as_ref()
            .expect("worker output")
            .exit_code,
        Some(7)
    );

    assert_eq!(timed_out.task.task_id, timed_out_task_id);
    assert_eq!(timed_out.worker.final_status, WorkerLaneStatus::TimedOut);
    assert_eq!(timed_out.task.status, TASK_FAILED_STATE);
    assert_eq!(
        timed_out
            .task
            .worker_output
            .as_ref()
            .expect("worker output")
            .status,
        "timed_out"
    );

    assert_eq!(cancelled.task.task_id, cancelled_task_id);
    assert_eq!(cancelled.worker.final_status, WorkerLaneStatus::Cancelled);
    assert_eq!(cancelled.task.status, TASK_CANCELLED_STATE);
    assert_eq!(
        cancelled.task.lease.as_ref().expect("lease slot").status,
        TASK_CANCELLED_STATE
    );
    assert!(
        runtime
            .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
                "queue-worker-empty",
                CodexWorkerLaneRequest::new(
                    "placeholder",
                    CodexWorkerLaneFixture::succeeded("empty"),
                ),
            ))?
            .is_none()
    );

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
    assert_eq!(diff_event.payload["repo_path"], worktree_path);
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
fn worker_lane_diff_inspect_report_captures_success_diff_and_is_read_only()
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
    let task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write worker lane diff inspect evidence",
    )?;

    let mut worker =
        CodexWorkerLaneRequest::new_subprocess("placeholder", fake_runner_command(script_name));
    worker.budget.max_stdout_bytes = 4;
    runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "worker-lane-inspect-worker",
            worker,
        ))?
        .expect("queued task should run");

    let store = PostgresEventStore::connect(&database_url)?;
    let events_before = runtime.events_for_session(started.session_id)?;
    let task_before = runtime.show_task(started.session_id, task_id)?;
    let queue_before = store.task_queue_records(Some(started.session_id))?;
    let report = runtime.inspect_worker_lane_diff(
        WorkerLaneDiffInspectRequest::new(started.session_id)
            .with_task_id(task_id)
            .with_payload_limit_bytes(32),
    )?;

    assert_eq!(report.scope, "task");
    assert_eq!(report.task_id, Some(task_id));
    assert_eq!(report.event_count, events_before.len());
    assert_eq!(report.lane_count, 1);
    assert_eq!(report.diff_count, 1);
    assert_eq!(report.source_of_truth, "EventLog");
    assert_eq!(report.projection_kind, "worker_lane_diff_inspect_report");

    let lane = &report.lanes[0];
    assert_eq!(lane.task_id, Some(task_id));
    assert_eq!(lane.lane_kind, "codex_cli");
    assert_eq!(lane.request_status.as_deref(), Some("requested"));
    assert_eq!(
        lane.policy.as_ref().map(|policy| policy.decision.as_str()),
        Some("allow")
    );
    assert_eq!(lane.current_state.as_deref(), Some("succeeded"));
    assert!(lane.is_terminal);
    assert_eq!(lane.terminal_state.as_deref(), Some("succeeded"));
    assert_eq!(
        lane.workspace.session_repo_path.as_deref(),
        Some(repo_path.as_str())
    );
    assert_eq!(
        lane.workspace.task_repo_path.as_deref(),
        Some(repo_path.as_str())
    );
    assert!(lane.workspace.allocated_worktree_path.is_some());
    let observation = lane.observation.as_ref().expect("worker observation");
    assert_eq!(observation.status, "succeeded");
    assert_eq!(observation.exit_code, Some(0));
    assert_eq!(observation.stdout.text, "0123");
    assert_eq!(observation.stderr.text, "abcd");
    assert_eq!(observation.usage_confidence, "unknown");

    let diff = &report.diffs[0];
    assert_eq!(diff.task_id, Some(task_id));
    assert_eq!(diff.files_changed, 1);
    assert_eq!(diff.paths, vec!["README.md".to_owned()]);
    assert_eq!(lane.workspace.diff_repo_path, diff.repo_path);
    assert!(diff.git_status.text.contains("README.md"));
    assert!(diff.git_diff.truncated);
    assert!(diff.git_diff.text.len() <= 32);

    let events_after = runtime.events_for_session(started.session_id)?;
    let task_after = runtime.show_task(started.session_id, task_id)?;
    let queue_after = store.task_queue_records(Some(started.session_id))?;
    assert_eq!(events_after.len(), events_before.len());
    assert_eq!(task_after, task_before);
    assert_eq!(queue_after, queue_before);

    Ok(())
}

#[test]
fn worker_lane_diff_inspect_report_captures_failure_timeout_and_cancellation()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let (failure_script_name, failure_script) = fake_failure_runner_script();
    let (timeout_script_name, timeout_script) = fake_timeout_runner_script();
    let repo = git_fixture_repo_with_files(&[
        (failure_script_name, failure_script),
        (timeout_script_name, timeout_script),
    ])?;
    let repo_path = repo.display().to_string();
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started = runtime.start_session(StartSessionRequest::new(repo_path))?;

    runtime.run_codex_worker_lane(
        started.session_id,
        CodexWorkerLaneRequest::new_subprocess(
            "failing inspect task",
            fake_runner_command(failure_script_name),
        ),
    )?;

    let mut timeout_request = CodexWorkerLaneRequest::new_subprocess(
        "timeout inspect task",
        fake_runner_command(timeout_script_name),
    );
    timeout_request.timeout_ms = 20;
    runtime.run_codex_worker_lane(started.session_id, timeout_request)?;

    let mut cancelled_request = CodexWorkerLaneRequest::new_subprocess(
        "cancelled inspect task",
        CodexWorkerSubprocess::new("missing-runner-should-not-start", Vec::new()),
    );
    cancelled_request.cancellation_requested = true;
    runtime.run_codex_worker_lane(started.session_id, cancelled_request)?;

    let report = runtime.inspect_worker_lane_diff(
        WorkerLaneDiffInspectRequest::new(started.session_id).with_payload_limit_bytes(128),
    )?;

    assert_eq!(report.lane_count, 3);
    assert_eq!(report.diff_count, 0);
    let statuses = report
        .lanes
        .iter()
        .map(|lane| lane.terminal_state.as_deref().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(statuses.contains(&"failed"));
    assert!(statuses.contains(&"timed_out"));
    assert!(statuses.contains(&"cancelled"));
    assert!(report.lanes.iter().any(|lane| {
        lane.failure_reason.is_some()
            && lane
                .observation
                .as_ref()
                .is_some_and(|observation| observation.exit_code == Some(7))
    }));
    assert!(report.lanes.iter().any(|lane| {
        lane.timeout_reason.is_some()
            && lane
                .observation
                .as_ref()
                .is_some_and(|observation| observation.status == "timed_out")
    }));
    assert!(
        report
            .lanes
            .iter()
            .any(|lane| lane.cancellation_reason.is_some()
                && lane.observation.as_ref().is_some_and(|observation| {
                    observation.stderr.text.contains("cancelled before start")
                }))
    );

    Ok(())
}

#[test]
fn task_scoped_commit_uses_existing_worktree_diff_repo() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let (script_name, script) = fake_success_runner_script();
    let task_repo = git_fixture_repo()?;
    let existing_worktree = git_fixture_repo_with_files(&[(script_name, script)])?;
    let task_repo_commits_before = git_commit_count(&task_repo)?;
    let existing_commits_before = git_commit_count(&existing_worktree)?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started =
        runtime.start_session(StartSessionRequest::new(task_repo.display().to_string()))?;
    let task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write a task diff in an explicit existing worktree",
    )?;

    let mut worker =
        CodexWorkerLaneRequest::new_subprocess("placeholder", fake_runner_command(script_name));
    worker.workspace =
        CodexWorkerLaneWorkspace::existing_worktree(existing_worktree.display().to_string());

    let result = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "existing-worktree-worker",
            worker,
        ))?
        .expect("queued task should run in existing worktree");
    assert_eq!(result.task.status, PENDING_COMMIT_APPROVAL_STATE);

    runtime.approve_task_pending_diff(started.session_id, task_id)?;
    let committed = runtime.commit_approved_task_diff(
        started.session_id,
        task_id,
        "Commit explicit existing worktree diff",
    )?;
    let events = runtime.events_for_session(started.session_id)?;
    let diff_event = events
        .iter()
        .find(|event| event.event_type == EventType::DiffRecorded)
        .expect("diff event");
    let commit_started = events
        .iter()
        .find(|event| event.event_type == EventType::CommitStarted)
        .expect("commit started event");
    let commit_succeeded = events
        .iter()
        .find(|event| event.event_type == EventType::CommitSucceeded)
        .expect("commit succeeded event");

    assert_eq!(committed.status, COMMITTED_STATE);
    assert_eq!(
        git_commit_count(&existing_worktree)?,
        existing_commits_before + 1
    );
    assert_eq!(git_commit_count(&task_repo)?, task_repo_commits_before);
    assert_eq!(git_text(&existing_worktree, &["status", "--short"])?, "");
    assert_eq!(git_text(&task_repo, &["status", "--short"])?, "");
    assert_eq!(
        diff_event.payload["repo_path"],
        existing_worktree.display().to_string()
    );
    assert_eq!(
        commit_started.payload["repo_path"],
        existing_worktree.display().to_string()
    );
    assert_eq!(
        commit_succeeded.payload["repo_path"],
        existing_worktree.display().to_string()
    );

    Ok(())
}

#[test]
fn task_scoped_commit_failure_uses_existing_worktree_diff_repo()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let (script_name, script) = fake_success_runner_script();
    let task_repo = git_fixture_repo()?;
    let existing_worktree = git_fixture_repo_with_files(&[(script_name, script)])?;
    let task_repo_commits_before = git_commit_count(&task_repo)?;
    let existing_commits_before = git_commit_count(&existing_worktree)?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let started =
        runtime.start_session(StartSessionRequest::new(task_repo.display().to_string()))?;
    let task_id = create_and_enqueue_task(
        &mut runtime,
        started.session_id,
        "write a task diff in an explicit existing worktree that later disappears",
    )?;

    let mut worker =
        CodexWorkerLaneRequest::new_subprocess("placeholder", fake_runner_command(script_name));
    worker.workspace =
        CodexWorkerLaneWorkspace::existing_worktree(existing_worktree.display().to_string());

    let result = runtime
        .run_next_leased_codex_worker_task(LeasedCodexWorkerTaskRequest::new(
            "existing-worktree-failing-commit-worker",
            worker,
        ))?
        .expect("queued task should run in existing worktree");
    assert_eq!(result.task.status, PENDING_COMMIT_APPROVAL_STATE);

    run_git(&existing_worktree, &["checkout", "--", "."])?;
    run_git(&existing_worktree, &["clean", "-fd"])?;
    runtime.approve_task_pending_diff(started.session_id, task_id)?;
    let failed = runtime.commit_approved_task_diff(
        started.session_id,
        task_id,
        "Commit missing explicit existing worktree diff",
    )?;
    let events = runtime.events_for_session(started.session_id)?;
    let diff_event = events
        .iter()
        .find(|event| event.event_type == EventType::DiffRecorded)
        .expect("diff event");
    let commit_started = events
        .iter()
        .find(|event| event.event_type == EventType::CommitStarted)
        .expect("commit started event");
    let commit_failed = events
        .iter()
        .find(|event| event.event_type == EventType::CommitFailed)
        .expect("commit failed event");

    assert_eq!(failed.status, COMMIT_FAILED_STATE);
    assert_eq!(
        git_commit_count(&existing_worktree)?,
        existing_commits_before
    );
    assert_eq!(git_commit_count(&task_repo)?, task_repo_commits_before);
    assert_eq!(git_text(&existing_worktree, &["status", "--short"])?, "");
    assert_eq!(git_text(&task_repo, &["status", "--short"])?, "");
    assert!(
        failed
            .commit
            .as_ref()
            .expect("commit slot")
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("approved diff has no local changes"))
    );
    assert_eq!(
        diff_event.payload["repo_path"],
        existing_worktree.display().to_string()
    );
    assert_eq!(
        commit_started.payload["repo_path"],
        existing_worktree.display().to_string()
    );
    assert_eq!(
        commit_failed.payload["repo_path"],
        existing_worktree.display().to_string()
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
fn approved_pending_diff_commits_through_harness_handoff() -> Result<(), Box<dyn std::error::Error>>
{
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = git_fixture_repo_with_files(&[("AGENTS.md", "Use deterministic agent fixtures.")])?;
    let commit_count_before = git_commit_count(&repo)?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let session_id = pending_approval_session_for_repo(
        &mut runtime,
        &repo,
        "write the commit handoff fake patch",
    )?;

    assert_eq!(git_commit_count(&repo)?, commit_count_before);
    runtime.approve_pending_diff(session_id)?;
    assert_eq!(git_commit_count(&repo)?, commit_count_before);

    let commit = runtime.commit_approved_diff(session_id, "Commit approved fake patch")?;
    let events = runtime.events_for_session(session_id)?;

    assert_eq!(commit.state, "committed");
    assert!(
        commit
            .commit_sha
            .as_deref()
            .is_some_and(|sha| sha.len() == 40)
    );
    assert_eq!(commit.failure_reason, None);
    assert_eq!(git_commit_count(&repo)?, commit_count_before + 1);
    assert_eq!(git_text(&repo, &["status", "--short"])?, "");
    assert_eq!(
        events.last().expect("last event").event_type,
        EventType::CommitSucceeded
    );
    assert_eq!(
        events.last().expect("last event").payload["commit_sha"],
        commit.commit_sha.as_deref().expect("commit sha")
    );
    let event_count_after_commit = events.len();
    let duplicate_commit = runtime
        .commit_approved_diff(session_id, "Should not commit twice")
        .expect_err("committed task cannot be committed again");
    assert!(duplicate_commit.to_string().contains("already committed"));
    assert_eq!(
        runtime.events_for_session(session_id)?.len(),
        event_count_after_commit
    );

    Ok(())
}

#[test]
fn commit_handoff_requires_approved_pending_diff() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let repo = git_fixture_repo_with_files(&[("AGENTS.md", "Use deterministic agent fixtures.")])?;
    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let pending_session_id = pending_approval_session_for_repo(
        &mut runtime,
        &repo,
        "write the unapproved commit handoff fake patch",
    )?;
    let pending_event_count = runtime.events_for_session(pending_session_id)?.len();

    let pending_error = runtime
        .commit_approved_diff(pending_session_id, "Should not commit")
        .expect_err("pending diff should not commit");
    assert!(
        pending_error
            .to_string()
            .contains("approved diff is required")
    );
    assert_eq!(
        runtime.events_for_session(pending_session_id)?.len(),
        pending_event_count
    );

    runtime.reject_pending_diff(pending_session_id, "not safe")?;
    let rejected_event_count = runtime.events_for_session(pending_session_id)?.len();
    let rejected_error = runtime
        .commit_approved_diff(pending_session_id, "Should not commit")
        .expect_err("rejected diff should not commit");
    assert!(
        rejected_error
            .to_string()
            .contains("current state is rejected")
    );
    assert_eq!(
        runtime.events_for_session(pending_session_id)?.len(),
        rejected_event_count
    );

    let empty_repo = fixture_repo()?;
    let missing =
        runtime.start_session(StartSessionRequest::new(empty_repo.display().to_string()))?;
    let missing_event_count = runtime.events_for_session(missing.session_id)?.len();
    let missing_error = runtime
        .commit_approved_diff(missing.session_id, "Should not commit")
        .expect_err("missing diff should not commit");
    assert!(
        missing_error
            .to_string()
            .contains("pending diff was not recorded")
    );
    assert_eq!(
        runtime.events_for_session(missing.session_id)?.len(),
        missing_event_count
    );

    Ok(())
}

#[test]
fn commit_handoff_records_git_failure_reason() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    harness_runtime::apply_database_migrations(&database_url)?;

    let mut runtime = Runtime::connect_postgres(&database_url)?;
    let session_id = pending_approval_session(&mut runtime)?;
    runtime.approve_pending_diff(session_id)?;

    let commit = runtime.commit_approved_diff(session_id, "Commit should fail")?;
    let events = runtime.events_for_session(session_id)?;

    assert_eq!(commit.state, "commit_failed");
    assert_eq!(commit.commit_sha, None);
    assert!(
        commit
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("git"))
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::CommitStarted)
    );
    assert_eq!(
        events.last().expect("last event").event_type,
        EventType::CommitFailed
    );
    assert!(
        events.last().expect("last event").payload["failure_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("git"))
    );
    let event_count_after_failure = events.len();
    let duplicate_commit = runtime
        .commit_approved_diff(session_id, "Retry failed commit")
        .expect_err("failed commit handoff cannot be committed again");
    assert!(
        duplicate_commit
            .to_string()
            .contains("already commit_failed")
    );
    assert_eq!(
        runtime.events_for_session(session_id)?.len(),
        event_count_after_failure
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

fn current_time_ms_for_test() -> Result<i64, Box<dyn std::error::Error>> {
    Ok(i64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?)
}

fn create_and_enqueue_task(
    runtime: &mut Runtime,
    session_id: Uuid,
    input: &str,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    create_and_enqueue_task_with_max_retries(runtime, session_id, input, 1)
}

fn create_and_enqueue_task_with_max_retries(
    runtime: &mut Runtime,
    session_id: Uuid,
    input: &str,
    max_retries: i32,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let task = runtime.create_task(session_id, CreateTaskRequest::new(input))?;
    let enqueued = runtime.enqueue_task_with_max_retries(session_id, task.task_id, max_retries)?;

    assert_eq!(enqueued.task_id, task.task_id);
    assert_eq!(enqueued.status, TASK_QUEUED_STATE);

    Ok(task.task_id)
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
    pending_approval_session_for_repo(runtime, &repo, "write the approval state fake patch")
}

fn pending_approval_session_for_repo(
    runtime: &mut Runtime,
    repo: &Path,
    task: &str,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let started = runtime.start_session(StartSessionRequest::new(repo.display().to_string()))?;
    runtime.run_small_coding_task(
        started.session_id,
        SmallCodingTaskRequest {
            task: task.to_owned(),
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

fn git_commit_count(root: &Path) -> Result<usize, Box<dyn std::error::Error>> {
    Ok(git_text(root, &["rev-list", "--count", "HEAD"])?
        .trim()
        .parse()?)
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
