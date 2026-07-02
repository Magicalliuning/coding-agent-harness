use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

#[test]
fn cli_reports_architecture_data_flow_and_storage_text_and_json()
-> Result<(), Box<dyn std::error::Error>> {
    let bin = env!("CARGO_BIN_EXE_harness-cli");

    let architecture = Command::new(bin)
        .args(["report", "architecture"])
        .output()?;
    assert!(architecture.status.success());
    let architecture_stdout = String::from_utf8(architecture.stdout)?;
    assert!(architecture_stdout.contains("report_id=runtime_architecture"));
    assert!(architecture_stdout.contains("node_count="));
    assert!(architecture_stdout.contains("node_1_id=runtime"));
    assert!(architecture_stdout.contains("node_2_id=policy_gate"));
    assert!(architecture_stdout.contains("boundary_adrs=ADR-0001,ADR-0002,ADR-0004,ADR-0007"));
    assert!(architecture_stdout.contains("non_goal_0_id=web_ui"));

    let architecture_json = Command::new(bin)
        .args(["report", "architecture", "--json"])
        .output()?;
    assert!(architecture_json.status.success());
    let architecture_json_stdout = String::from_utf8(architecture_json.stdout)?;
    let architecture: Value = serde_json::from_str(&architecture_json_stdout)?;
    assert_eq!(architecture["report_id"], "runtime_architecture");
    assert!(
        architecture["nodes"]
            .as_array()
            .expect("architecture nodes")
            .iter()
            .any(|node| node["id"] == "local_governed_codex_cli_worker")
    );
    assert!(
        architecture["non_goals"]
            .as_array()
            .expect("architecture non-goals")
            .iter()
            .any(|non_goal| non_goal["id"] == "replace_eventlog")
    );

    let data_flow = Command::new(bin).args(["report", "data-flow"]).output()?;
    assert!(data_flow.status.success());
    let data_flow_stdout = String::from_utf8(data_flow.stdout)?;
    assert!(data_flow_stdout.contains("report_id=runtime_data_flow"));
    assert!(data_flow_stdout.contains("step_0_id=task_creation"));
    assert!(data_flow_stdout.contains("step_5_id=policy_decision"));
    assert!(data_flow_stdout.contains("step_12_event_types=commit.succeeded,commit.failed"));

    let data_flow_json = Command::new(bin)
        .args(["report", "data-flow", "--json"])
        .output()?;
    assert!(data_flow_json.status.success());
    let data_flow_json_stdout = String::from_utf8(data_flow_json.stdout)?;
    let data_flow: Value = serde_json::from_str(&data_flow_json_stdout)?;
    assert_eq!(data_flow["projection_kind"], "runtime_data_flow_report");
    assert_eq!(data_flow["steps"][0]["id"], "task_creation");
    assert_eq!(data_flow["steps"][5]["component_id"], "policy_gate");
    assert_eq!(data_flow["steps"][12]["event_types"][1], "commit.failed");

    let storage = Command::new(bin).args(["report", "storage"]).output()?;
    assert!(storage.status.success());
    let storage_stdout = String::from_utf8(storage.stdout)?;
    assert!(storage_stdout.contains("report_id=runtime_storage"));
    assert!(storage_stdout.contains("table_0_name=harness_runtime.event_log"));
    assert!(storage_stdout.contains("table_1_name=harness_runtime.task_queue"));
    assert!(storage_stdout.contains("projection_0_id=session_projection"));

    let storage_json = Command::new(bin)
        .args(["report", "storage", "--json"])
        .output()?;
    assert!(storage_json.status.success());
    let storage_json_stdout = String::from_utf8(storage_json.stdout)?;
    let storage: Value = serde_json::from_str(&storage_json_stdout)?;
    assert_eq!(storage["projection_kind"], "runtime_storage_report");
    assert_eq!(storage["tables"][0]["role"], "append_only_source_of_truth");
    assert_eq!(storage["tables"][1]["role"], "runtime_scheduling_state");
    assert!(
        storage["projections"]
            .as_array()
            .expect("storage projections")
            .iter()
            .any(|projection| projection["id"] == "approval_commit_projection")
    );

    Ok(())
}

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
fn cli_creates_lists_and_shows_session_tasks() -> Result<(), Box<dyn std::error::Error>> {
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

    let first = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the first CLI task",
            "--max-bytes",
            "4096",
            "--max-files",
            "8",
            "--max-skill-files",
            "2",
            "--focus",
            "agent",
            "--focus",
            "task",
            "--max-output-tokens",
            "384",
        ])
        .output()?;
    assert!(first.status.success());

    let first_stdout = String::from_utf8(first.stdout)?;
    let first_task_id = value_for_key(&first_stdout, "task_id").expect("task_id output");
    assert!(first_stdout.contains("task_status=created"));
    assert!(first_stdout.contains("task_input=write the first CLI task"));
    assert!(first_stdout.contains("task_worker_lane_kind=codex_cli_worker_lane"));
    assert!(first_stdout.contains("task_context_max_bytes=4096"));
    assert!(first_stdout.contains("task_context_max_files=8"));
    assert!(first_stdout.contains("task_context_max_skill_files=2"));
    assert!(first_stdout.contains("task_focus_terms=agent,task"));
    assert!(first_stdout.contains("task_max_output_tokens=384"));
    assert!(first_stdout.contains("task_worktree_path="));
    assert!(first_stdout.contains("task_approval_state="));
    assert!(first_stdout.contains("event_count=2"));

    let second = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the second CLI task",
        ])
        .output()?;
    assert!(second.status.success());

    let second_stdout = String::from_utf8(second.stdout)?;
    let second_task_id = value_for_key(&second_stdout, "task_id").expect("second task_id output");
    assert_ne!(first_task_id, second_task_id);

    let list = Command::new(bin)
        .args([
            "session",
            "task",
            "list",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(list.status.success());

    let list_stdout = String::from_utf8(list.stdout)?;
    assert!(list_stdout.contains("task_count=2"));
    assert!(list_stdout.contains(&format!("task_ids={first_task_id},{second_task_id}")));
    assert!(list_stdout.contains(&format!("task_0_id={first_task_id}")));
    assert!(list_stdout.contains("task_0_status=created"));
    assert!(list_stdout.contains("task_0_input=write the first CLI task"));
    assert!(list_stdout.contains(&format!("task_1_id={second_task_id}")));
    assert!(list_stdout.contains("task_1_input=write the second CLI task"));

    let show = Command::new(bin)
        .args([
            "session",
            "task",
            "show",
            session_id,
            first_task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(show.status.success());

    let show_stdout = String::from_utf8(show.stdout)?;
    assert!(show_stdout.contains(&format!("task_id={first_task_id}")));
    assert!(show_stdout.contains("task_status=created"));
    assert!(show_stdout.contains("task_input=write the first CLI task"));
    assert!(show_stdout.contains("event_count=3"));

    Ok(())
}

#[test]
fn cli_inspects_session_text_and_json_without_mutating_state()
-> Result<(), Box<dyn std::error::Error>> {
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

    let first = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the first inspect CLI task",
        ])
        .output()?;
    assert!(first.status.success());

    let first_stdout = String::from_utf8(first.stdout)?;
    let first_task_id = value_for_key(&first_stdout, "task_id").expect("task_id output");

    let second = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the second inspect CLI task",
        ])
        .output()?;
    assert!(second.status.success());

    let enqueue = Command::new(bin)
        .args([
            "session",
            "task",
            "enqueue",
            session_id,
            first_task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(enqueue.status.success());

    let before_show = Command::new(bin)
        .args([
            "session",
            "show",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(before_show.status.success());
    let before_show_stdout = String::from_utf8(before_show.stdout)?;
    assert!(before_show_stdout.contains("event_count=4"));

    let before_task = Command::new(bin)
        .args([
            "session",
            "task",
            "show",
            session_id,
            first_task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(before_task.status.success());
    let before_task_stdout = String::from_utf8(before_task.stdout)?;
    assert!(before_task_stdout.contains("task_queue_status=queued"));

    let inspect_text = Command::new(bin)
        .args([
            "session",
            "inspect",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(inspect_text.status.success());
    let inspect_text_stdout = String::from_utf8(inspect_text.stdout)?;
    assert!(inspect_text_stdout.contains(&format!("session_id={session_id}")));
    assert!(inspect_text_stdout.contains("session_status=started"));
    assert!(inspect_text_stdout.contains(&format!("session_repo_path={repo_path}")));
    assert!(inspect_text_stdout.contains("event_count=4"));
    assert!(inspect_text_stdout.contains("latest_event_sequence=4"));
    assert!(inspect_text_stdout.contains("latest_event_type=task.enqueued"));
    assert!(inspect_text_stdout.contains("task_count=2"));
    assert!(inspect_text_stdout.contains("created:1"));
    assert!(inspect_text_stdout.contains("queued:1"));
    assert!(inspect_text_stdout.contains("source_of_truth=EventLog"));
    assert!(inspect_text_stdout.contains("projection_kind=derived_from_eventlog"));

    let inspect_json = Command::new(bin)
        .args([
            "session",
            "inspect",
            session_id,
            "--database-url",
            &database_url,
            "--json",
        ])
        .output()?;
    assert!(inspect_json.status.success());
    let inspect_json_stdout = String::from_utf8(inspect_json.stdout)?;
    let json: Value = serde_json::from_str(&inspect_json_stdout)?;
    assert_eq!(json["session_id"], session_id);
    assert_eq!(json["repo_path"], repo_path);
    assert_eq!(json["status"], "started");
    assert_eq!(json["event_count"], 4);
    assert_eq!(json["latest_event_sequence"], 4);
    assert_eq!(json["latest_event_type"], "task.enqueued");
    assert_eq!(json["task_count"], 2);
    assert_eq!(json["source_of_truth"], "EventLog");
    assert_eq!(json["projection_kind"], "derived_from_eventlog");
    let counts = json["task_status_counts"]
        .as_array()
        .expect("status counts array");
    assert!(
        counts
            .iter()
            .any(|count| count["status"] == "created" && count["count"] == 1)
    );
    assert!(
        counts
            .iter()
            .any(|count| count["status"] == "queued" && count["count"] == 1)
    );

    let after_show = Command::new(bin)
        .args([
            "session",
            "show",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(after_show.status.success());
    let after_show_stdout = String::from_utf8(after_show.stdout)?;
    assert!(after_show_stdout.contains("event_count=4"));

    let after_task = Command::new(bin)
        .args([
            "session",
            "task",
            "show",
            session_id,
            first_task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(after_task.status.success());
    let after_task_stdout = String::from_utf8(after_task.stdout)?;
    assert!(after_task_stdout.contains("task_queue_status=queued"));

    Ok(())
}

#[test]
fn cli_session_inspect_reports_missing_and_malformed_session_ids()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    let bin = env!("CARGO_BIN_EXE_harness-cli");

    let migrate = Command::new(bin)
        .args(["migrate", "--database-url", &database_url])
        .output()?;
    assert!(migrate.status.success());

    let missing_session_id = uuid::Uuid::new_v4().to_string();
    let missing = Command::new(bin)
        .args([
            "session",
            "inspect",
            &missing_session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(!missing.status.success());
    let missing_stderr = String::from_utf8(missing.stderr)?;
    assert!(missing_stderr.contains(&format!("session not found: {missing_session_id}")));

    let malformed = Command::new(bin)
        .args([
            "session",
            "inspect",
            "not-a-session-id",
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(!malformed.status.success());
    let malformed_stderr = String::from_utf8(malformed.stderr)?;
    assert!(malformed_stderr.contains("session id must be a UUID"));

    Ok(())
}

#[test]
fn cli_inspects_bounded_eventlog_timeline_text_and_json() -> Result<(), Box<dyn std::error::Error>>
{
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
    let large_task = format!("write the bounded timeline task {}", "x".repeat(640));

    let first = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            &large_task,
        ])
        .output()?;
    assert!(first.status.success());
    let first_stdout = String::from_utf8(first.stdout)?;
    let first_task_id = value_for_key(&first_stdout, "task_id").expect("task_id output");

    let second = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the unrelated timeline task",
        ])
        .output()?;
    assert!(second.status.success());
    let second_stdout = String::from_utf8(second.stdout)?;
    let second_task_id = value_for_key(&second_stdout, "task_id").expect("task_id output");

    let before_show = Command::new(bin)
        .args([
            "session",
            "show",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(before_show.status.success());
    let before_show_stdout = String::from_utf8(before_show.stdout)?;
    assert!(before_show_stdout.contains("event_count=3"));

    let timeline_text = Command::new(bin)
        .args([
            "session",
            "timeline",
            session_id,
            "--database-url",
            &database_url,
            "--payload-bytes",
            "48",
        ])
        .output()?;
    assert!(timeline_text.status.success());
    let timeline_text_stdout = String::from_utf8(timeline_text.stdout)?;
    assert!(timeline_text_stdout.contains(&format!("session_id={session_id}")));
    assert!(timeline_text_stdout.contains("timeline_scope=session"));
    assert!(timeline_text_stdout.contains("event_count=3"));
    assert!(timeline_text_stdout.contains("payload_limit_bytes=48"));
    assert!(timeline_text_stdout.contains("event_0_sequence=1"));
    assert!(timeline_text_stdout.contains("event_0_type=session.started"));
    assert!(timeline_text_stdout.contains("event_0_schema_version=1"));
    assert!(timeline_text_stdout.contains("event_1_type=task.created"));
    assert!(timeline_text_stdout.contains("event_1_payload_truncated=true"));
    assert!(timeline_text_stdout.contains("source_of_truth=EventLog"));
    assert!(timeline_text_stdout.contains("projection_kind=bounded_eventlog_timeline"));

    let timeline_json = Command::new(bin)
        .args([
            "session",
            "timeline",
            session_id,
            "--database-url",
            &database_url,
            "--task-id",
            first_task_id,
            "--payload-bytes",
            "48",
            "--json",
        ])
        .output()?;
    assert!(timeline_json.status.success());
    let timeline_json_stdout = String::from_utf8(timeline_json.stdout)?;
    let json: Value = serde_json::from_str(&timeline_json_stdout)?;
    assert_eq!(json["session_id"], session_id);
    assert_eq!(json["task_id"], first_task_id);
    assert_eq!(json["event_count"], 1);
    assert_eq!(json["payload_limit_bytes"], 48);
    assert_eq!(json["source_of_truth"], "EventLog");
    assert_eq!(json["projection_kind"], "bounded_eventlog_timeline");
    let events = json["events"].as_array().expect("timeline events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["sequence"], 2);
    assert_eq!(events[0]["event_type"], "task.created");
    assert_eq!(events[0]["schema_version"], 1);
    assert_eq!(events[0]["task_id"], first_task_id);
    assert_eq!(events[0]["payload"]["truncated"], true);
    assert!(
        events[0]["payload"]["text"]
            .as_str()
            .is_some_and(|text| text.len() <= 48)
    );
    assert!(!timeline_json_stdout.contains(second_task_id));

    let missing_task = Command::new(bin)
        .args([
            "session",
            "timeline",
            session_id,
            "--database-url",
            &database_url,
            "--task-id",
            &uuid::Uuid::new_v4().to_string(),
        ])
        .output()?;
    assert!(!missing_task.status.success());
    let missing_task_stderr = String::from_utf8(missing_task.stderr)?;
    assert!(missing_task_stderr.contains("task not found"));

    let missing_session_id = uuid::Uuid::new_v4().to_string();
    let missing_session = Command::new(bin)
        .args([
            "session",
            "timeline",
            &missing_session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(!missing_session.status.success());
    let missing_session_stderr = String::from_utf8(missing_session.stderr)?;
    assert!(missing_session_stderr.contains(&format!("session not found: {missing_session_id}")));

    let after_show = Command::new(bin)
        .args([
            "session",
            "show",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(after_show.status.success());
    let after_show_stdout = String::from_utf8(after_show.stdout)?;
    assert!(after_show_stdout.contains("event_count=3"));

    Ok(())
}

#[test]
fn cli_inspects_task_text_and_json_without_mutating_state() -> Result<(), Box<dyn std::error::Error>>
{
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

    let create = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the inspectable CLI task",
            "--max-bytes",
            "4096",
            "--max-files",
            "8",
            "--max-skill-files",
            "2",
            "--focus",
            "inspect",
            "--max-output-tokens",
            "384",
        ])
        .output()?;
    assert!(create.status.success());
    let create_stdout = String::from_utf8(create.stdout)?;
    let task_id = value_for_key(&create_stdout, "task_id").expect("task_id output");

    let enqueue = Command::new(bin)
        .args([
            "session",
            "task",
            "enqueue",
            session_id,
            task_id,
            "--database-url",
            &database_url,
            "--max-retries",
            "2",
        ])
        .output()?;
    assert!(enqueue.status.success());

    let lease = Command::new(bin)
        .args([
            "session",
            "task",
            "lease-next",
            "--database-url",
            &database_url,
            "--worker-id",
            "cli-inspect-worker",
            "--lease-ms",
            "30000",
        ])
        .output()?;
    assert!(lease.status.success());
    let lease_stdout = String::from_utf8(lease.stdout)?;
    let lease_id = value_for_key(&lease_stdout, "task_lease_id").expect("lease id output");

    let before_show = Command::new(bin)
        .args([
            "session",
            "show",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(before_show.status.success());
    let before_show_stdout = String::from_utf8(before_show.stdout)?;
    assert!(before_show_stdout.contains("event_count=4"));

    let inspect_text = Command::new(bin)
        .args([
            "session",
            "task",
            "inspect",
            session_id,
            task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(inspect_text.status.success());
    let inspect_text_stdout = String::from_utf8(inspect_text.stdout)?;
    assert!(inspect_text_stdout.contains(&format!("session_id={session_id}")));
    assert!(inspect_text_stdout.contains(&format!("task_id={task_id}")));
    assert!(inspect_text_stdout.contains("task_status=leased"));
    assert!(inspect_text_stdout.contains("task_input_summary=write the inspectable CLI task"));
    assert!(inspect_text_stdout.contains("task_context_max_bytes=4096"));
    assert!(inspect_text_stdout.contains("task_context_max_files=8"));
    assert!(inspect_text_stdout.contains("task_context_max_skill_files=2"));
    assert!(inspect_text_stdout.contains("task_focus_terms=inspect"));
    assert!(inspect_text_stdout.contains("task_max_output_tokens=384"));
    assert!(inspect_text_stdout.contains("task_queue_status=leased"));
    assert!(inspect_text_stdout.contains("task_queue_worker_id=cli-inspect-worker"));
    assert!(inspect_text_stdout.contains(&format!("task_queue_lease_id={lease_id}")));
    assert!(inspect_text_stdout.contains("task_queue_max_retries=2"));
    assert!(inspect_text_stdout.contains(&format!("task_lease_id={lease_id}")));
    assert!(inspect_text_stdout.contains("task_lease_worker_id=cli-inspect-worker"));
    assert!(inspect_text_stdout.contains("task_event_count=3"));
    assert!(inspect_text_stdout.contains("task_event_latest_type=task.lease_acquired"));
    assert!(inspect_text_stdout.contains("source_of_truth=EventLog"));
    assert!(inspect_text_stdout.contains("projection_kind=task_inspect_report"));

    let inspect_json = Command::new(bin)
        .args([
            "session",
            "task",
            "inspect",
            session_id,
            task_id,
            "--database-url",
            &database_url,
            "--json",
        ])
        .output()?;
    assert!(inspect_json.status.success());
    let inspect_json_stdout = String::from_utf8(inspect_json.stdout)?;
    let json: Value = serde_json::from_str(&inspect_json_stdout)?;
    assert_eq!(json["session_id"], session_id);
    assert_eq!(json["task_id"], task_id);
    assert_eq!(json["status"], "leased");
    assert_eq!(
        json["input_summary"]["text"],
        "write the inspectable CLI task"
    );
    assert_eq!(json["worker_lane_kind"], "codex_cli_worker_lane");
    assert_eq!(json["context_budget"]["max_bytes"], 4096);
    assert_eq!(json["context_budget"]["max_files"], 8);
    assert_eq!(json["context_budget"]["max_skill_files"], 2);
    assert_eq!(json["max_output_tokens"], 384);
    assert_eq!(json["queue"]["status"], "leased");
    assert_eq!(json["queue"]["worker_id"], "cli-inspect-worker");
    assert_eq!(json["queue"]["lease_id"], lease_id);
    assert_eq!(json["queue"]["max_retries"], 2);
    assert_eq!(json["lease"]["worker_id"], "cli-inspect-worker");
    assert_eq!(json["event_summary"]["event_count"], 3);
    assert_eq!(
        json["event_summary"]["latest_event_type"],
        "task.lease_acquired"
    );
    assert_eq!(json["source_of_truth"], "EventLog");
    assert_eq!(json["projection_kind"], "task_inspect_report");

    let other_start = Command::new(bin)
        .args([
            "session",
            "start",
            "--repo",
            &repo_path,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(other_start.status.success());
    let other_start_stdout = String::from_utf8(other_start.stdout)?;
    let other_session_id =
        value_for_key(&other_start_stdout, "session_id").expect("other session id output");
    let outside_session = Command::new(bin)
        .args([
            "session",
            "task",
            "inspect",
            other_session_id,
            task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(!outside_session.status.success());
    let outside_session_stderr = String::from_utf8(outside_session.stderr)?;
    assert!(outside_session_stderr.contains("task not found"));

    let after_show = Command::new(bin)
        .args([
            "session",
            "show",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(after_show.status.success());
    let after_show_stdout = String::from_utf8(after_show.stdout)?;
    assert!(after_show_stdout.contains("event_count=4"));

    let after_task = Command::new(bin)
        .args([
            "session",
            "task",
            "show",
            session_id,
            task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(after_task.status.success());
    let after_task_stdout = String::from_utf8(after_task.stdout)?;
    assert!(after_task_stdout.contains("task_queue_status=leased"));
    assert!(after_task_stdout.contains(&format!("task_lease_id={lease_id}")));

    Ok(())
}

#[test]
fn cli_inspects_task_queue_text_and_json_without_mutating_state()
-> Result<(), Box<dyn std::error::Error>> {
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

    let empty = Command::new(bin)
        .args([
            "session",
            "task",
            "queue-inspect",
            "--session-id",
            session_id,
            "--database-url",
            &database_url,
            "--now-ms",
            "0",
        ])
        .output()?;
    assert!(empty.status.success());
    let empty_stdout = String::from_utf8(empty.stdout)?;
    assert!(empty_stdout.contains(&format!("session_id={session_id}")));
    assert!(empty_stdout.contains("queue_empty=true"));
    assert!(empty_stdout.contains("queue_total_count=0"));
    assert!(empty_stdout.contains("projection_kind=task_queue_inspect_report"));

    let create = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the queue inspectable CLI task",
        ])
        .output()?;
    assert!(create.status.success());
    let create_stdout = String::from_utf8(create.stdout)?;
    let task_id = value_for_key(&create_stdout, "task_id").expect("task_id output");

    let enqueue = Command::new(bin)
        .args([
            "session",
            "task",
            "enqueue",
            session_id,
            task_id,
            "--database-url",
            &database_url,
            "--max-retries",
            "2",
        ])
        .output()?;
    assert!(enqueue.status.success());

    let lease = Command::new(bin)
        .args([
            "session",
            "task",
            "lease-next",
            "--database-url",
            &database_url,
            "--worker-id",
            "cli-queue-worker",
            "--lease-ms",
            "30000",
        ])
        .output()?;
    assert!(lease.status.success());
    let lease_stdout = String::from_utf8(lease.stdout)?;
    let lease_id = value_for_key(&lease_stdout, "task_lease_id").expect("lease id output");
    let lease_deadline_ms =
        value_for_key(&lease_stdout, "task_lease_deadline_ms").expect("lease deadline output");

    let before_show = Command::new(bin)
        .args([
            "session",
            "show",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(before_show.status.success());
    let before_show_stdout = String::from_utf8(before_show.stdout)?;
    assert!(before_show_stdout.contains("event_count=4"));

    let inspect_text = Command::new(bin)
        .args([
            "session",
            "task",
            "queue-inspect",
            "--session-id",
            session_id,
            "--database-url",
            &database_url,
            "--now-ms",
            "0",
        ])
        .output()?;
    assert!(inspect_text.status.success());
    let inspect_text_stdout = String::from_utf8(inspect_text.stdout)?;
    assert!(inspect_text_stdout.contains("queue_empty=false"));
    assert!(inspect_text_stdout.contains("queue_total_count=1"));
    assert!(inspect_text_stdout.contains("queue_active_leased_count=1"));
    assert!(inspect_text_stdout.contains("queue_expired_looking_leased_count=0"));
    assert!(inspect_text_stdout.contains("queue_status_counts=leased:1"));
    assert!(inspect_text_stdout.contains("queue_status_class_counts=active_leased:1"));
    assert!(inspect_text_stdout.contains("queue_task_status_counts=leased:1"));
    assert!(inspect_text_stdout.contains("queue_lease_state_counts=active:1"));
    assert!(inspect_text_stdout.contains(&format!("queue_task_0_id={task_id}")));
    assert!(inspect_text_stdout.contains("queue_task_0_task_status=leased"));
    assert!(inspect_text_stdout.contains("queue_task_0_queue_status=leased"));
    assert!(inspect_text_stdout.contains("queue_task_0_status_class=active_leased"));
    assert!(inspect_text_stdout.contains("queue_task_0_lease_state=active"));
    assert!(inspect_text_stdout.contains("queue_task_0_worker_id=cli-queue-worker"));
    assert!(inspect_text_stdout.contains(&format!("queue_task_0_lease_id={lease_id}")));
    assert!(inspect_text_stdout.contains(&format!(
        "queue_task_0_lease_deadline_ms={lease_deadline_ms}"
    )));
    assert!(inspect_text_stdout.contains("queue_task_0_retry_count=0"));
    assert!(inspect_text_stdout.contains("queue_task_0_max_retries=2"));
    assert!(inspect_text_stdout.contains("source_of_truth=task_queue+EventLog"));
    assert!(inspect_text_stdout.contains("projection_kind=task_queue_inspect_report"));

    let inspect_json = Command::new(bin)
        .args([
            "session",
            "task",
            "queue-inspect",
            "--session-id",
            session_id,
            "--database-url",
            &database_url,
            "--now-ms",
            &i64::MAX.to_string(),
            "--json",
        ])
        .output()?;
    assert!(inspect_json.status.success());
    let inspect_json_stdout = String::from_utf8(inspect_json.stdout)?;
    let json: Value = serde_json::from_str(&inspect_json_stdout)?;
    assert_eq!(json["session_id"], session_id);
    assert_eq!(json["total_count"], 1);
    assert_eq!(json["active_leased_count"], 0);
    assert_eq!(json["expired_looking_leased_count"], 1);
    assert_eq!(json["queue_status_counts"][0]["status"], "leased");
    assert_eq!(
        json["status_class_counts"][0]["status"],
        "expired_looking_leased"
    );
    assert_eq!(json["lease_state_counts"][0]["status"], "expired_looking");
    assert_eq!(json["tasks"][0]["task_id"], task_id);
    assert_eq!(json["tasks"][0]["queue_status"], "leased");
    assert_eq!(json["tasks"][0]["status_class"], "expired_looking_leased");
    assert_eq!(json["tasks"][0]["lease_state"], "expired_looking");
    assert_eq!(json["tasks"][0]["worker_id"], "cli-queue-worker");
    assert_eq!(json["tasks"][0]["lease_id"], lease_id);
    assert_eq!(json["tasks"][0]["retry_count"], 0);
    assert_eq!(json["tasks"][0]["max_retries"], 2);
    assert_eq!(json["source_of_truth"], "task_queue+EventLog");

    let after_show = Command::new(bin)
        .args([
            "session",
            "show",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(after_show.status.success());
    let after_show_stdout = String::from_utf8(after_show.stdout)?;
    assert!(after_show_stdout.contains("event_count=4"));

    let after_task = Command::new(bin)
        .args([
            "session",
            "task",
            "show",
            session_id,
            task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(after_task.status.success());
    let after_task_stdout = String::from_utf8(after_task.stdout)?;
    assert!(after_task_stdout.contains("task_queue_status=leased"));
    assert!(after_task_stdout.contains(&format!("task_lease_id={lease_id}")));
    assert!(after_task_stdout.contains(&format!("task_lease_deadline_ms={lease_deadline_ms}")));

    Ok(())
}

#[test]
fn cli_enqueues_leases_and_completes_session_task() -> Result<(), Box<dyn std::error::Error>> {
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

    let create = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the queued CLI task",
        ])
        .output()?;
    assert!(create.status.success());

    let create_stdout = String::from_utf8(create.stdout)?;
    let task_id = value_for_key(&create_stdout, "task_id").expect("task_id output");
    assert!(create_stdout.contains("task_status=created"));
    assert!(create_stdout.contains("task_queue_status="));

    let enqueue = Command::new(bin)
        .args([
            "session",
            "task",
            "enqueue",
            session_id,
            task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(enqueue.status.success());

    let enqueue_stdout = String::from_utf8(enqueue.stdout)?;
    assert!(enqueue_stdout.contains("task_status=queued"));
    assert!(enqueue_stdout.contains("task_queue_status=queued"));
    assert!(enqueue_stdout.contains("task_queue_reason=task enqueued for worker execution"));

    let lease = Command::new(bin)
        .args([
            "session",
            "task",
            "lease-next",
            "--database-url",
            &database_url,
            "--worker-id",
            "cli-worker",
            "--lease-ms",
            "30000",
        ])
        .output()?;
    assert!(lease.status.success());

    let lease_stdout = String::from_utf8(lease.stdout)?;
    let lease_id = value_for_key(&lease_stdout, "task_lease_id").expect("lease id output");
    assert!(lease_stdout.contains("task_leased=true"));
    assert!(lease_stdout.contains("task_status=leased"));
    assert!(lease_stdout.contains("task_queue_status=leased"));
    assert!(lease_stdout.contains("task_lease_worker_id=cli-worker"));
    assert!(lease_stdout.contains("task_lease_status=leased"));
    assert!(lease_stdout.contains("task_lease_deadline_ms="));

    let complete = Command::new(bin)
        .args([
            "session",
            "task",
            "complete",
            task_id,
            "--database-url",
            &database_url,
            "--lease-id",
            lease_id,
            "--reason",
            "CLI worker completed task",
        ])
        .output()?;
    assert!(complete.status.success());

    let complete_stdout = String::from_utf8(complete.stdout)?;
    assert!(complete_stdout.contains("task_status=completed"));
    assert!(complete_stdout.contains("task_queue_status=completed"));
    assert!(complete_stdout.contains("task_lease_status=completed"));
    assert!(complete_stdout.contains("task_lease_reason=CLI worker completed task"));

    Ok(())
}

#[test]
fn cli_renews_expires_retries_and_stops_task_leases() -> Result<(), Box<dyn std::error::Error>> {
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

    let create = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the retrying CLI task",
        ])
        .output()?;
    assert!(create.status.success());

    let create_stdout = String::from_utf8(create.stdout)?;
    let task_id = value_for_key(&create_stdout, "task_id").expect("task_id output");

    let enqueue = Command::new(bin)
        .args([
            "session",
            "task",
            "enqueue",
            session_id,
            task_id,
            "--database-url",
            &database_url,
            "--max-retries",
            "1",
        ])
        .output()?;
    assert!(enqueue.status.success());

    let first_lease = Command::new(bin)
        .args([
            "session",
            "task",
            "lease-next",
            "--database-url",
            &database_url,
            "--worker-id",
            "cli-worker-1",
            "--lease-ms",
            "1",
        ])
        .output()?;
    assert!(first_lease.status.success());

    let first_lease_stdout = String::from_utf8(first_lease.stdout)?;
    let first_lease_id =
        value_for_key(&first_lease_stdout, "task_lease_id").expect("first lease id output");
    assert!(first_lease_stdout.contains("task_status=leased"));
    assert!(first_lease_stdout.contains("task_queue_retry_count=0"));
    assert!(first_lease_stdout.contains("task_queue_max_retries=1"));

    let heartbeat = Command::new(bin)
        .args([
            "session",
            "task",
            "heartbeat",
            task_id,
            "--database-url",
            &database_url,
            "--lease-id",
            first_lease_id,
            "--worker-id",
            "cli-worker-1",
            "--lease-ms",
            "60000",
        ])
        .output()?;
    assert!(heartbeat.status.success());

    let heartbeat_stdout = String::from_utf8(heartbeat.stdout)?;
    assert!(heartbeat_stdout.contains("task_status=leased"));
    assert!(heartbeat_stdout.contains("task_lease_status=leased"));
    assert!(heartbeat_stdout.contains("task_lease_reason=task lease renewed"));

    let expire_retry = Command::new(bin)
        .args([
            "session",
            "task",
            "expire-leases",
            "--database-url",
            &database_url,
            "--now-ms",
            "9223372036854775807",
        ])
        .output()?;
    assert!(expire_retry.status.success());

    let expire_retry_stdout = String::from_utf8(expire_retry.stdout)?;
    assert!(expire_retry_stdout.contains("expired_task_count=1"));
    assert!(expire_retry_stdout.contains("task_status=retry_queued"));
    assert!(expire_retry_stdout.contains("task_queue_retry_count=1"));
    assert!(expire_retry_stdout.contains("task_queue_stop_reason=lease_expired"));
    assert!(expire_retry_stdout.contains("task_lease_status=expired"));

    let second_lease = Command::new(bin)
        .args([
            "session",
            "task",
            "lease-next",
            "--database-url",
            &database_url,
            "--worker-id",
            "cli-worker-2",
        ])
        .output()?;
    assert!(second_lease.status.success());

    let second_lease_stdout = String::from_utf8(second_lease.stdout)?;
    assert!(second_lease_stdout.contains(&format!("task_id={task_id}")));
    assert!(second_lease_stdout.contains("task_lease_worker_id=cli-worker-2"));
    assert!(second_lease_stdout.contains("task_queue_retry_count=1"));

    let expire_stop = Command::new(bin)
        .args([
            "session",
            "task",
            "expire-leases",
            "--database-url",
            &database_url,
            "--now-ms",
            "9223372036854775807",
        ])
        .output()?;
    assert!(expire_stop.status.success());

    let expire_stop_stdout = String::from_utf8(expire_stop.stdout)?;
    assert!(expire_stop_stdout.contains("expired_task_count=1"));
    assert!(expire_stop_stdout.contains("task_status=stopped"));
    assert!(expire_stop_stdout.contains("task_queue_stop_reason=max_retries_exceeded"));
    assert!(expire_stop_stdout.contains("task_lease_status=stopped"));

    let empty_lease = Command::new(bin)
        .args([
            "session",
            "task",
            "lease-next",
            "--database-url",
            &database_url,
            "--worker-id",
            "cli-worker-late",
        ])
        .output()?;
    assert!(empty_lease.status.success());
    assert!(String::from_utf8(empty_lease.stdout)?.contains("task_leased=false"));

    Ok(())
}

#[test]
fn cli_runs_next_queued_task_through_codex_worker() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    let bin = env!("CARGO_BIN_EXE_harness-cli");
    let repo = fixture_repo()?;
    let (script_name, script) = fake_arg_check_runner_script();
    write_file(&repo, "AGENTS.md", "Use deterministic agent fixtures.")?;
    write_file(&repo, "README.md", "fixture repository\n")?;
    write_file(&repo, script_name, script)?;
    init_git_repo(&repo)?;

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

    let create = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the queued CLI worker task",
        ])
        .output()?;
    assert!(create.status.success());

    let create_stdout = String::from_utf8(create.stdout)?;
    let task_id = value_for_key(&create_stdout, "task_id").expect("task id output");

    let enqueue = Command::new(bin)
        .args([
            "session",
            "task",
            "enqueue",
            session_id,
            task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(enqueue.status.success());

    let mut args = vec![
        "session".to_owned(),
        "task".to_owned(),
        "run-next-codex-worker".to_owned(),
        "--worker-id".to_owned(),
        "cli-queue-worker".to_owned(),
        "--timeout-ms".to_owned(),
        "120000".to_owned(),
        "--max-stdout-bytes".to_owned(),
        "4".to_owned(),
        "--".to_owned(),
    ];
    args.extend(fake_runner_cli_command(script_name));
    args.extend([
        "--database-url".to_owned(),
        "postgres://child.invalid/harness".to_owned(),
        "--child-flag".to_owned(),
        "passed".to_owned(),
    ]);
    let before_ms = current_time_ms_for_test()?;
    let run = Command::new(bin)
        .args(args)
        .env("HARNESS_DATABASE_URL", &database_url)
        .output()?;
    assert!(run.status.success());

    let run_stdout = String::from_utf8(run.stdout)?;
    let lease_deadline_ms: i64 = value_for_key(&run_stdout, "task_lease_deadline_ms")
        .expect("task lease deadline output")
        .parse()?;
    assert!(run_stdout.contains("task_leased=true"));
    assert!(run_stdout.contains("worker_final_status=succeeded"));
    assert!(run_stdout.contains("worker_pending_commit_state=pending_commit_approval"));
    assert!(run_stdout.contains("task_status=pending_commit_approval"));
    assert!(run_stdout.contains("task_queue_status=completed"));
    assert!(run_stdout.contains("task_lease_status=completed"));
    assert!(run_stdout.contains("task_worker_status=succeeded"));
    assert!(run_stdout.contains("task_diff_paths=README.md"));
    assert!(run_stdout.contains("task_approval_state=pending_commit_approval"));
    assert!(
        lease_deadline_ms >= before_ms + 121_000,
        "lease deadline {lease_deadline_ms} should cover timeout plus margin from {before_ms}"
    );
    assert_eq!(git_status(&repo)?, "");

    Ok(())
}

#[test]
fn cli_inspects_worker_lane_diff_text_and_json() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    let bin = env!("CARGO_BIN_EXE_harness-cli");
    let repo = fixture_repo()?;
    let (script_name, script) = fake_success_runner_script();
    write_file(&repo, "AGENTS.md", "Use deterministic agent fixtures.")?;
    write_file(&repo, "README.md", "fixture repository\n")?;
    write_file(&repo, script_name, script)?;
    init_git_repo(&repo)?;

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

    let create = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the worker inspect CLI task",
        ])
        .output()?;
    assert!(create.status.success());

    let create_stdout = String::from_utf8(create.stdout)?;
    let task_id = value_for_key(&create_stdout, "task_id").expect("task id output");

    let enqueue = Command::new(bin)
        .args([
            "session",
            "task",
            "enqueue",
            session_id,
            task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(enqueue.status.success());

    let mut run_args = vec![
        "session".to_owned(),
        "task".to_owned(),
        "run-next-codex-worker".to_owned(),
        "--database-url".to_owned(),
        database_url.clone(),
        "--worker-id".to_owned(),
        "cli-worker-inspect-worker".to_owned(),
        "--".to_owned(),
    ];
    run_args.extend(fake_runner_cli_command(script_name));
    let run = Command::new(bin).args(run_args).output()?;
    assert!(run.status.success());

    let before_show = Command::new(bin)
        .args([
            "session",
            "show",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(before_show.status.success());
    let before_show_stdout = String::from_utf8(before_show.stdout)?;
    let before_event_count =
        value_for_key(&before_show_stdout, "event_count").expect("event_count output");

    let inspect_text = Command::new(bin)
        .args([
            "session",
            "worker",
            "inspect",
            session_id,
            "--database-url",
            &database_url,
            "--payload-bytes",
            "4",
        ])
        .output()?;
    assert!(inspect_text.status.success());
    let inspect_text_stdout = String::from_utf8(inspect_text.stdout)?;
    assert!(inspect_text_stdout.contains(&format!("session_id={session_id}")));
    assert!(inspect_text_stdout.contains("worker_lane_scope=session"));
    assert!(inspect_text_stdout.contains("lane_count=1"));
    assert!(inspect_text_stdout.contains("diff_count=1"));
    assert!(inspect_text_stdout.contains("lane_0_kind=codex_cli"));
    assert!(inspect_text_stdout.contains("lane_0_policy_decision=allow"));
    assert!(inspect_text_stdout.contains("lane_0_terminal_state=succeeded"));
    assert!(inspect_text_stdout.contains("lane_0_stdout_truncated=true"));
    assert!(inspect_text_stdout.contains("lane_0_stdout_summary=0123"));
    assert!(inspect_text_stdout.contains("diff_0_paths=README.md"));
    assert!(inspect_text_stdout.contains("diff_0_git_diff_truncated=true"));
    assert!(inspect_text_stdout.contains("projection_kind=worker_lane_diff_inspect_report"));

    let inspect_json = Command::new(bin)
        .args([
            "session",
            "worker",
            "inspect",
            session_id,
            "--database-url",
            &database_url,
            "--task-id",
            task_id,
            "--payload-bytes",
            "4",
            "--json",
        ])
        .output()?;
    assert!(inspect_json.status.success());
    let inspect_json_stdout = String::from_utf8(inspect_json.stdout)?;
    let json: Value = serde_json::from_str(&inspect_json_stdout)?;
    assert_eq!(json["session_id"], session_id);
    assert_eq!(json["task_id"], task_id);
    assert_eq!(json["scope"], "task");
    assert_eq!(json["lane_count"], 1);
    assert_eq!(json["diff_count"], 1);
    assert_eq!(json["projection_kind"], "worker_lane_diff_inspect_report");
    assert_eq!(json["lanes"][0]["task_id"], task_id);
    assert_eq!(json["lanes"][0]["policy"]["decision"], "allow");
    assert_eq!(json["lanes"][0]["terminal_state"], "succeeded");
    assert_eq!(json["lanes"][0]["observation"]["stdout"]["text"], "0123");
    assert_eq!(json["lanes"][0]["observation"]["stdout"]["truncated"], true);
    assert_eq!(json["diffs"][0]["paths"][0], "README.md");
    assert_eq!(json["diffs"][0]["git_diff"]["truncated"], true);

    let after_show = Command::new(bin)
        .args([
            "session",
            "show",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(after_show.status.success());
    let after_show_stdout = String::from_utf8(after_show.stdout)?;
    assert_eq!(
        value_for_key(&after_show_stdout, "event_count").expect("event_count output"),
        before_event_count
    );

    Ok(())
}

#[test]
fn cli_approves_and_commits_queued_task_scope() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    let bin = env!("CARGO_BIN_EXE_harness-cli");
    let repo = fixture_repo()?;
    let (script_name, script) = fake_success_runner_script();
    write_file(&repo, "AGENTS.md", "Use deterministic agent fixtures.")?;
    write_file(&repo, "README.md", "fixture repository\n")?;
    write_file(&repo, script_name, script)?;
    init_git_repo(&repo)?;

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

    let create = Command::new(bin)
        .args([
            "session",
            "task",
            "create",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the queued approval CLI worker task",
        ])
        .output()?;
    assert!(create.status.success());

    let create_stdout = String::from_utf8(create.stdout)?;
    let task_id = value_for_key(&create_stdout, "task_id").expect("task id output");

    let enqueue = Command::new(bin)
        .args([
            "session",
            "task",
            "enqueue",
            session_id,
            task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(enqueue.status.success());

    let mut run_args = vec![
        "session".to_owned(),
        "task".to_owned(),
        "run-next-codex-worker".to_owned(),
        "--database-url".to_owned(),
        database_url.clone(),
        "--worker-id".to_owned(),
        "cli-task-approval-worker".to_owned(),
        "--max-stdout-bytes".to_owned(),
        "4".to_owned(),
        "--".to_owned(),
    ];
    run_args.extend(fake_runner_cli_command(script_name));
    let run = Command::new(bin).args(run_args).output()?;
    assert!(run.status.success());
    let run_stdout = String::from_utf8(run.stdout)?;
    assert!(run_stdout.contains("task_status=pending_commit_approval"));

    let session_approve = Command::new(bin)
        .args([
            "session",
            "approval",
            "approve",
            session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(!session_approve.status.success());
    assert!(String::from_utf8(session_approve.stderr)?.contains("pending diff was not recorded"));

    let approve = Command::new(bin)
        .args([
            "session",
            "task",
            "approve",
            session_id,
            task_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(approve.status.success());
    let approve_stdout = String::from_utf8(approve.stdout)?;
    assert!(approve_stdout.contains("task_status=approved"));
    assert!(approve_stdout.contains("task_approval_state=approved"));

    let commit = Command::new(bin)
        .args([
            "session",
            "task",
            "commit",
            session_id,
            task_id,
            "--database-url",
            &database_url,
            "--message",
            "Commit queued task from CLI",
        ])
        .output()?;
    assert!(commit.status.success());
    let commit_stdout = String::from_utf8(commit.stdout)?;
    assert!(commit_stdout.contains("task_status=committed"));
    assert!(commit_stdout.contains("task_commit_state=committed"));
    assert!(commit_stdout.contains("task_commit_sha="));

    Ok(())
}

#[test]
fn cli_codex_acceptance_reports_explicit_skip_when_unavailable()
-> Result<(), Box<dyn std::error::Error>> {
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

    let acceptance = Command::new(bin)
        .args([
            "session",
            "codex-acceptance",
            session_id,
            "--database-url",
            &database_url,
            "--codex-program",
            "missing-codex-cli-for-harness-test",
        ])
        .output()?;
    assert!(acceptance.status.success());

    let stdout = String::from_utf8(acceptance.stdout)?;
    assert!(stdout.contains("codex_acceptance_status=skipped"));
    assert!(stdout.contains("codex_available=false"));
    assert!(stdout.contains("codex_authenticated=false"));
    assert!(stdout.contains("codex_skipped_reason=Codex CLI executable could not be started"));
    assert!(!stdout.contains("worker_lane_id="));

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

#[test]
fn cli_coding_task_stops_pending_commit_approval() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    let bin = env!("CARGO_BIN_EXE_harness-cli");
    let repo = fixture_repo()?;
    write_file(&repo, "AGENTS.md", "Use deterministic agent fixtures.")?;
    init_git_repo(&repo)?;
    let commit_count_before = git_commit_count(&repo)?;

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

    let task = Command::new(bin)
        .args([
            "session",
            "coding-task",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the verified CLI fake patch",
            "--max-bytes",
            "4096",
            "--focus",
            "agent",
            "--",
            "cargo",
            "--version",
        ])
        .output()?;
    assert!(task.status.success());

    let task_stdout = String::from_utf8(task.stdout)?;
    let written = fs::read_to_string(repo.join(".harness/fake-agent-turn.md"))?;
    let commit_count_after = git_commit_count(&repo)?;
    assert!(task_stdout.contains("patch_path=.harness/fake-agent-turn.md"));
    assert!(task_stdout.contains("patch_applied=true"));
    assert!(task_stdout.contains("verification_decision=allow"));
    assert!(task_stdout.contains("verification_executed=true"));
    assert!(task_stdout.contains("verification_exit_code=0"));
    assert!(task_stdout.contains("diff_files_changed=1"));
    assert!(task_stdout.contains("diff_insertions="));
    assert!(task_stdout.contains("diff_deletions=0"));
    assert!(task_stdout.contains("event_replay_total=12"));
    assert!(task_stdout.contains("event_replay_last=commit.approval_pending"));
    assert!(task_stdout.contains("token_prompt="));
    assert!(task_stdout.contains("token_completion="));
    assert!(task_stdout.contains("token_total="));
    assert!(task_stdout.contains("final_state=pending_commit_approval"));
    assert!(task_stdout.contains("event_count=12"));
    assert!(written.contains("write the verified CLI fake patch"));
    assert_eq!(commit_count_before, commit_count_after);

    Ok(())
}

#[test]
fn cli_approval_show_approve_rejects_pending_diff() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    let bin = env!("CARGO_BIN_EXE_harness-cli");

    let migrate = Command::new(bin)
        .args(["migrate", "--database-url", &database_url])
        .output()?;
    assert!(migrate.status.success());

    let (_approved_repo, approved_session_id) =
        start_cli_pending_approval(bin, &database_url, "write the approved CLI patch")?;

    let show = Command::new(bin)
        .args([
            "session",
            "approval",
            "show",
            &approved_session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(show.status.success());

    let show_stdout = String::from_utf8(show.stdout)?;
    assert!(show_stdout.contains("approval_state=pending_commit_approval"));
    assert!(show_stdout.contains("diff_paths=.harness/fake-agent-turn.md"));

    let approve = Command::new(bin)
        .args([
            "session",
            "approval",
            "approve",
            &approved_session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(approve.status.success());

    let approve_stdout = String::from_utf8(approve.stdout)?;
    assert!(approve_stdout.contains("approval_state=approved"));
    assert!(approve_stdout.contains("approval_rejection_reason="));

    let duplicate_approve = Command::new(bin)
        .args([
            "session",
            "approval",
            "approve",
            &approved_session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(!duplicate_approve.status.success());
    assert!(String::from_utf8(duplicate_approve.stderr)?.contains("already approved"));

    let (_rejected_repo, rejected_session_id) =
        start_cli_pending_approval(bin, &database_url, "write the rejected CLI patch")?;

    let reject = Command::new(bin)
        .args([
            "session",
            "approval",
            "reject",
            &rejected_session_id,
            "--database-url",
            &database_url,
            "--reason",
            "not the requested change",
        ])
        .output()?;
    assert!(reject.status.success());

    let reject_stdout = String::from_utf8(reject.stdout)?;
    assert!(reject_stdout.contains("approval_state=rejected"));
    assert!(reject_stdout.contains("approval_rejection_reason=not the requested change"));

    Ok(())
}

#[test]
fn cli_inspects_approval_commit_text_and_json_without_mutating_state()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    let bin = env!("CARGO_BIN_EXE_harness-cli");

    let migrate = Command::new(bin)
        .args(["migrate", "--database-url", &database_url])
        .output()?;
    assert!(migrate.status.success());

    let (_repo, session_id) =
        start_cli_pending_approval(bin, &database_url, "write the inspectable approval patch")?;

    let before_show = Command::new(bin)
        .args([
            "session",
            "show",
            &session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(before_show.status.success());
    let before_show_stdout = String::from_utf8(before_show.stdout)?;
    assert!(before_show_stdout.contains("event_count=12"));

    let before_approval = Command::new(bin)
        .args([
            "session",
            "approval",
            "show",
            &session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(before_approval.status.success());
    let before_approval_stdout = String::from_utf8(before_approval.stdout)?;
    assert!(before_approval_stdout.contains("approval_state=pending_commit_approval"));

    let inspect_text = Command::new(bin)
        .args([
            "session",
            "approval",
            "inspect",
            &session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(inspect_text.status.success());
    let inspect_text_stdout = String::from_utf8(inspect_text.stdout)?;
    assert!(inspect_text_stdout.contains(&format!("session_id={session_id}")));
    assert!(inspect_text_stdout.contains("task_id="));
    assert!(inspect_text_stdout.contains("approval_commit_scope=session"));
    assert!(inspect_text_stdout.contains("pending_approval_count=1"));
    assert!(inspect_text_stdout.contains("approval_decision_count=0"));
    assert!(inspect_text_stdout.contains("commit_handoff_count=0"));
    assert!(
        inspect_text_stdout
            .contains("pending_0_summary=verification passed; awaiting human commit approval")
    );
    assert!(inspect_text_stdout.contains("pending_0_diff_paths=.harness/fake-agent-turn.md"));
    assert!(inspect_text_stdout.contains("source_of_truth=EventLog"));
    assert!(inspect_text_stdout.contains("projection_kind=approval_commit_inspect_report"));

    let inspect_json = Command::new(bin)
        .args([
            "session",
            "approval",
            "inspect",
            &session_id,
            "--database-url",
            &database_url,
            "--json",
        ])
        .output()?;
    assert!(inspect_json.status.success());
    let inspect_json_stdout = String::from_utf8(inspect_json.stdout)?;
    let json: Value = serde_json::from_str(&inspect_json_stdout)?;
    assert_eq!(json["session_id"], session_id);
    assert_eq!(json["scope"], "session");
    assert_eq!(json["pending_count"], 1);
    assert_eq!(json["decision_count"], 0);
    assert_eq!(json["commit_count"], 0);
    assert_eq!(
        json["pending_approvals"][0]["summary"],
        "verification passed; awaiting human commit approval"
    );
    assert_eq!(
        json["pending_approvals"][0]["diff"]["paths"][0],
        ".harness/fake-agent-turn.md"
    );
    assert_eq!(json["source_of_truth"], "EventLog");

    let after_show = Command::new(bin)
        .args([
            "session",
            "show",
            &session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(after_show.status.success());
    let after_show_stdout = String::from_utf8(after_show.stdout)?;
    assert!(after_show_stdout.contains("event_count=12"));

    let after_approval = Command::new(bin)
        .args([
            "session",
            "approval",
            "show",
            &session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(after_approval.status.success());
    let after_approval_stdout = String::from_utf8(after_approval.stdout)?;
    assert_eq!(after_approval_stdout, before_approval_stdout);

    Ok(())
}

#[test]
fn cli_commit_handoff_requires_approval_and_commits_approved_diff()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    let bin = env!("CARGO_BIN_EXE_harness-cli");

    let migrate = Command::new(bin)
        .args(["migrate", "--database-url", &database_url])
        .output()?;
    assert!(migrate.status.success());

    let pending_repo = fixture_repo()?;
    write_file(
        &pending_repo,
        "AGENTS.md",
        "Use deterministic agent fixtures.",
    )?;
    init_git_repo(&pending_repo)?;
    let pending_commit_count_before: usize = git_commit_count(&pending_repo)?.parse()?;
    let pending_session_id = start_cli_pending_approval_in_repo(
        bin,
        &database_url,
        &pending_repo,
        "write the unapproved CLI patch",
    )?;
    assert_eq!(
        git_commit_count(&pending_repo)?.parse::<usize>()?,
        pending_commit_count_before
    );

    let unapproved_commit = Command::new(bin)
        .args([
            "session",
            "commit",
            &pending_session_id,
            "--database-url",
            &database_url,
            "--message",
            "Should not commit",
        ])
        .output()?;
    assert!(!unapproved_commit.status.success());
    assert!(String::from_utf8(unapproved_commit.stderr)?.contains("approved diff is required"),);
    assert_eq!(
        git_commit_count(&pending_repo)?.parse::<usize>()?,
        pending_commit_count_before
    );

    let approved_repo = fixture_repo()?;
    write_file(
        &approved_repo,
        "AGENTS.md",
        "Use deterministic agent fixtures.",
    )?;
    init_git_repo(&approved_repo)?;
    let approved_commit_count_before: usize = git_commit_count(&approved_repo)?.parse()?;
    let approved_session_id = start_cli_pending_approval_in_repo(
        bin,
        &database_url,
        &approved_repo,
        "write the approved CLI commit patch",
    )?;
    assert_eq!(
        git_commit_count(&approved_repo)?.parse::<usize>()?,
        approved_commit_count_before
    );

    let approve = Command::new(bin)
        .args([
            "session",
            "approval",
            "approve",
            &approved_session_id,
            "--database-url",
            &database_url,
        ])
        .output()?;
    assert!(approve.status.success());
    assert_eq!(
        git_commit_count(&approved_repo)?.parse::<usize>()?,
        approved_commit_count_before
    );

    let commit = Command::new(bin)
        .args([
            "session",
            "commit",
            &approved_session_id,
            "--database-url",
            &database_url,
            "--message",
            "Commit approved CLI patch",
        ])
        .output()?;
    assert!(commit.status.success());

    let commit_stdout = String::from_utf8(commit.stdout)?;
    let commit_sha = value_for_key(&commit_stdout, "commit_sha").expect("commit sha output");
    assert!(commit_stdout.contains("commit_state=committed"));
    assert!(commit_stdout.contains("commit_message=Commit approved CLI patch"));
    assert!(commit_stdout.contains("commit_failure_reason="));
    assert!(commit_stdout.contains("event_count=15"));
    assert_eq!(commit_sha.len(), 40);
    assert_eq!(
        git_commit_count(&approved_repo)?.parse::<usize>()?,
        approved_commit_count_before + 1
    );
    assert_eq!(git_status(&approved_repo)?, "");

    Ok(())
}

#[test]
fn cli_recovers_fixture_task_with_bounded_loop() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = database_url() else {
        return Ok(());
    };

    let bin = env!("CARGO_BIN_EXE_harness-cli");
    let repo = v0_acceptance_fixture_repo()?;
    init_git_repo(&repo)?;
    let commit_count_before = git_commit_count(&repo)?;

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

    let recover = Command::new(bin)
        .args([
            "session",
            "recover-fixture",
            session_id,
            "--database-url",
            &database_url,
            "--task",
            "write the CLI recovery fixture patch",
            "--max-bytes",
            "4096",
            "--focus",
            "agent",
            "--max-recovery-rounds",
            "2",
            "--max-repair-bytes",
            "4096",
        ])
        .output()?;
    assert!(recover.status.success());

    let recover_stdout = String::from_utf8(recover.stdout)?;
    let written = fs::read_to_string(repo.join(".harness/fake-agent-turn.md"))?;
    let commit_count_after = git_commit_count(&repo)?;
    assert!(recover_stdout.contains("recovery_classification=fixture_missing_recovery_marker"));
    assert!(recover_stdout.contains("recovery_plan="));
    assert!(recover_stdout.contains("recovery_attempts=1"));
    assert!(recover_stdout.contains("recovery_retries=1"));
    assert!(recover_stdout.contains("recovery_stop_reason=recovered"));
    assert!(recover_stdout.contains("recovery_max_rounds=2"));
    assert!(recover_stdout.contains("recovery_used_repair_bytes="));
    assert!(recover_stdout.contains("verification_decision=allow"));
    assert!(recover_stdout.contains("verification_executed=true"));
    assert!(recover_stdout.contains("verification_exit_code=0"));
    assert!(recover_stdout.contains("diff_files_changed=1"));
    assert!(recover_stdout.contains("diff_insertions="));
    assert!(recover_stdout.contains("diff_deletions="));
    assert!(recover_stdout.contains("diff_paths=.harness/fake-agent-turn.md"));
    assert!(recover_stdout.contains("event_replay_total=22"));
    assert!(recover_stdout.contains("event_replay_last=recovery.stopped"));
    assert!(recover_stdout.contains("token_prompt="));
    assert!(recover_stdout.contains("token_completion="));
    assert!(recover_stdout.contains("token_total="));
    assert!(recover_stdout.contains("token_max_output=256"));
    assert!(recover_stdout.contains("final_state=pending_commit_approval"));
    assert!(recover_stdout.contains("event_count=22"));
    assert!(written.contains("recovered=true"));
    assert_eq!(commit_count_before, commit_count_after);

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

fn current_time_ms_for_test() -> Result<i64, Box<dyn std::error::Error>> {
    Ok(i64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?)
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

fn v0_acceptance_fixture_repo() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let source = workspace_root()?.join("fixtures/v0-acceptance");
    let target = fixture_repo()?;

    copy_dir(&source, &target)?;
    Ok(target)
}

fn start_cli_pending_approval(
    bin: &str,
    database_url: &str,
    task: &str,
) -> Result<(PathBuf, String), Box<dyn std::error::Error>> {
    let repo = fixture_repo()?;
    write_file(&repo, "AGENTS.md", "Use deterministic agent fixtures.")?;
    let session_id = start_cli_pending_approval_in_repo(bin, database_url, &repo, task)?;

    Ok((repo, session_id))
}

fn start_cli_pending_approval_in_repo(
    bin: &str,
    database_url: &str,
    repo: &Path,
    task: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let repo_path = repo.display().to_string();

    let start = Command::new(bin)
        .args([
            "session",
            "start",
            "--repo",
            &repo_path,
            "--database-url",
            database_url,
        ])
        .output()?;
    assert!(start.status.success());

    let start_stdout = String::from_utf8(start.stdout)?;
    let session_id = value_for_key(&start_stdout, "session_id")
        .expect("session_id output")
        .to_owned();

    let coding_task = Command::new(bin)
        .args([
            "session",
            "coding-task",
            &session_id,
            "--database-url",
            database_url,
            "--task",
            task,
            "--max-bytes",
            "4096",
            "--focus",
            "agent",
            "--",
            "cargo",
            "--version",
        ])
        .output()?;
    assert!(coding_task.status.success());

    Ok(session_id)
}

fn workspace_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?)
}

fn copy_dir(source: &Path, target: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(target)?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());

        if entry.file_type()?.is_dir() {
            copy_dir(&source_path, &target_path)?;
        } else {
            fs::copy(source_path, target_path)?;
        }
    }

    Ok(())
}

fn fake_runner_cli_command(script_name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        vec!["cmd".to_owned(), "/C".to_owned(), script_name.to_owned()]
    }
    #[cfg(not(windows))]
    {
        vec!["sh".to_owned(), script_name.to_owned()]
    }
}

#[cfg(windows)]
fn fake_success_runner_script() -> (&'static str, &'static str) {
    (
        "fake-success-runner.cmd",
        "@echo off\r\necho 0123456789\r\necho changed by CLI fake runner>> README.md\r\nexit /B 0\r\n",
    )
}

#[cfg(not(windows))]
fn fake_success_runner_script() -> (&'static str, &'static str) {
    (
        "fake-success-runner.sh",
        "#!/bin/sh\nprintf '0123456789\\n'\nprintf 'changed by CLI fake runner\\n' >> README.md\nexit 0\n",
    )
}

#[cfg(windows)]
fn fake_arg_check_runner_script() -> (&'static str, &'static str) {
    (
        "fake-arg-check-runner.cmd",
        "@echo off\r\nif NOT \"%~1\"==\"--database-url\" exit /B 2\r\nif NOT \"%~2\"==\"postgres://child.invalid/harness\" exit /B 3\r\nif NOT \"%~3\"==\"--child-flag\" exit /B 4\r\nif NOT \"%~4\"==\"passed\" exit /B 5\r\necho 0123456789\r\necho changed by CLI fake runner>> README.md\r\nexit /B 0\r\n",
    )
}

#[cfg(not(windows))]
fn fake_arg_check_runner_script() -> (&'static str, &'static str) {
    (
        "fake-arg-check-runner.sh",
        "#!/bin/sh\n[ \"$1\" = \"--database-url\" ] || exit 2\n[ \"$2\" = \"postgres://child.invalid/harness\" ] || exit 3\n[ \"$3\" = \"--child-flag\" ] || exit 4\n[ \"$4\" = \"passed\" ] || exit 5\nprintf '0123456789\\n'\nprintf 'changed by CLI fake runner\\n' >> README.md\nexit 0\n",
    )
}

fn init_git_repo(repo: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let init = Command::new("git").arg("init").current_dir(repo).output()?;
    assert!(init.status.success());

    let add = Command::new("git")
        .args(["add", "."])
        .current_dir(repo)
        .output()?;
    assert!(add.status.success());

    let commit = Command::new("git")
        .args([
            "-c",
            "user.name=Coding Agent Harness Test",
            "-c",
            "user.email=harness-test@example.invalid",
            "commit",
            "-m",
            "initial fixture",
        ])
        .current_dir(repo)
        .output()?;
    assert!(commit.status.success());

    Ok(())
}

fn git_commit_count(repo: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(repo)
        .output()?;
    assert!(output.status.success());

    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn git_status(repo: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["status", "--short"])
        .current_dir(repo)
        .output()?;
    assert!(output.status.success());

    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
