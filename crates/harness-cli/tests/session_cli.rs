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
        "--database-url".to_owned(),
        database_url.clone(),
        "--worker-id".to_owned(),
        "cli-queue-worker".to_owned(),
        "--max-stdout-bytes".to_owned(),
        "4".to_owned(),
        "--".to_owned(),
    ];
    args.extend(fake_runner_cli_command(script_name));
    let run = Command::new(bin).args(args).output()?;
    assert!(run.status.success());

    let run_stdout = String::from_utf8(run.stdout)?;
    assert!(run_stdout.contains("task_leased=true"));
    assert!(run_stdout.contains("worker_final_status=succeeded"));
    assert!(run_stdout.contains("worker_pending_commit_state=pending_commit_approval"));
    assert!(run_stdout.contains("task_status=pending_commit_approval"));
    assert!(run_stdout.contains("task_queue_status=completed"));
    assert!(run_stdout.contains("task_lease_status=completed"));
    assert!(run_stdout.contains("task_worker_status=succeeded"));
    assert!(run_stdout.contains("task_diff_paths=README.md"));
    assert!(run_stdout.contains("task_approval_state=pending_commit_approval"));
    assert_eq!(git_status(&repo)?, "");

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
