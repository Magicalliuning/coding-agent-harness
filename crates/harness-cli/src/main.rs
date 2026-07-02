use std::env;
use std::path::PathBuf;

use harness_core::{HarnessError, HarnessResult};
use harness_runtime::{
    ApprovalProjection, CodexCliAcceptanceRequest, CodexCliAcceptanceResult,
    CodexCliAvailabilityRequest, CommitHandoffProjection, ContextBudget, CreateTaskRequest,
    DEFAULT_CODEX_CLI_ACCEPTANCE_TASK, DEFAULT_CODEX_CLI_ACCEPTANCE_TIMEOUT_MS,
    DEFAULT_CODEX_CLI_PROGRAM, DEFAULT_TASK_WORKER_LANE_KIND, FakeModelTurnRequest,
    FakeModelTurnResult, Runtime, SelfRecoveryLoopRequest, SelfRecoveryLoopResult,
    SessionContextCompileRequest, SessionContextCompileResult, SessionProjection,
    SmallCodingTaskRequest, SmallCodingTaskResult, StartSessionRequest, TaskProjection,
    VerificationCommandRequest, VerificationCommandResult,
};
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
        "task" => {
            let action = args
                .get(1)
                .ok_or_else(|| HarnessError::new("task action is required"))?;
            let session_id = args
                .get(2)
                .ok_or_else(|| HarnessError::new("session id is required"))?;
            let session_id = Uuid::parse_str(session_id)
                .map_err(|error| HarnessError::new(error.to_string()))?;
            let database_url = database_url_from_args(args.clone())?;
            let mut runtime = Runtime::connect_postgres(&database_url)?;

            match action.as_str() {
                "create" => {
                    let input = required_arg_value(&args, "--task")?;
                    let repo_path = optional_arg_value(&args, "--repo")
                        .map(canonical_repo_path)
                        .transpose()?;
                    let worker_lane_kind = optional_arg_value(&args, "--worker-lane")
                        .unwrap_or(DEFAULT_TASK_WORKER_LANE_KIND);
                    let max_output_tokens = optional_arg_value(&args, "--max-output-tokens")
                        .map(|value| parse_usize_arg("--max-output-tokens", value))
                        .transpose()?
                        .unwrap_or(256);
                    let request = CreateTaskRequest {
                        input: input.to_owned(),
                        repo_path,
                        worker_lane_kind: worker_lane_kind.to_owned(),
                        context: SessionContextCompileRequest {
                            budget: context_budget_from_args(&args)?,
                            focus_terms: repeated_arg_values(&args, "--focus"),
                        },
                        max_output_tokens,
                    };
                    let task = runtime.create_task(session_id, request)?;
                    print_task_projection(&task);
                    Ok(())
                }
                "list" => {
                    let tasks = runtime.list_tasks(session_id)?;
                    print_task_list(&tasks);
                    Ok(())
                }
                "show" => {
                    let task_id = args
                        .get(3)
                        .ok_or_else(|| HarnessError::new("task id is required"))?;
                    let task_id = Uuid::parse_str(task_id)
                        .map_err(|error| HarnessError::new(error.to_string()))?;
                    let task = runtime.show_task(session_id, task_id)?;
                    print_task_projection(&task);
                    Ok(())
                }
                other => Err(HarnessError::new(format!("unknown task action: {other}"))),
            }
        }
        "verify-command" => {
            let session_id = args
                .get(1)
                .ok_or_else(|| HarnessError::new("session id is required"))?;
            let session_id = Uuid::parse_str(session_id)
                .map_err(|error| HarnessError::new(error.to_string()))?;
            let database_url = database_url_from_args(args_before_separator(&args).to_vec())?;
            let command = command_after_separator(&args)?;
            let mut runtime = Runtime::connect_postgres(&database_url)?;
            let result = runtime.run_verification_command(
                session_id,
                VerificationCommandRequest::new(command[0].clone(), command[1..].to_vec()),
            )?;

            print_verification_result(&result);
            Ok(())
        }
        "compile-context" => {
            let session_id = args
                .get(1)
                .ok_or_else(|| HarnessError::new("session id is required"))?;
            let session_id = Uuid::parse_str(session_id)
                .map_err(|error| HarnessError::new(error.to_string()))?;
            let database_url = database_url_from_args(args.clone())?;
            let budget = context_budget_from_args(&args)?;
            let focus_terms = repeated_arg_values(&args, "--focus");
            let mut runtime = Runtime::connect_postgres(&database_url)?;
            let result = runtime.compile_session_context(
                session_id,
                SessionContextCompileRequest {
                    budget,
                    focus_terms,
                },
            )?;

            print_context_compile_result(&result);
            Ok(())
        }
        "fake-turn" => {
            let session_id = args
                .get(1)
                .ok_or_else(|| HarnessError::new("session id is required"))?;
            let session_id = Uuid::parse_str(session_id)
                .map_err(|error| HarnessError::new(error.to_string()))?;
            let database_url = database_url_from_args(args.clone())?;
            let task = required_arg_value(&args, "--task")?;
            let budget = context_budget_from_args(&args)?;
            let focus_terms = repeated_arg_values(&args, "--focus");
            let max_output_tokens = optional_arg_value(&args, "--max-output-tokens")
                .map(|value| parse_usize_arg("--max-output-tokens", value))
                .transpose()?
                .unwrap_or(256);
            let mut runtime = Runtime::connect_postgres(&database_url)?;
            let result = runtime.run_fake_model_turn(
                session_id,
                FakeModelTurnRequest {
                    task: task.to_owned(),
                    context: SessionContextCompileRequest {
                        budget,
                        focus_terms,
                    },
                    max_output_tokens,
                },
            )?;

            print_fake_model_turn_result(&result);
            Ok(())
        }
        "coding-task" => {
            let session_id = args
                .get(1)
                .ok_or_else(|| HarnessError::new("session id is required"))?;
            let session_id = Uuid::parse_str(session_id)
                .map_err(|error| HarnessError::new(error.to_string()))?;
            let database_url = database_url_from_args(args_before_separator(&args).to_vec())?;
            let task = required_arg_value(args_before_separator(&args), "--task")?;
            let budget = context_budget_from_args(args_before_separator(&args))?;
            let focus_terms = repeated_arg_values(args_before_separator(&args), "--focus");
            let max_output_tokens =
                optional_arg_value(args_before_separator(&args), "--max-output-tokens")
                    .map(|value| parse_usize_arg("--max-output-tokens", value))
                    .transpose()?
                    .unwrap_or(256);
            let command = command_after_separator(&args)?;
            let mut runtime = Runtime::connect_postgres(&database_url)?;
            let result = runtime.run_small_coding_task(
                session_id,
                SmallCodingTaskRequest {
                    task: task.to_owned(),
                    context: SessionContextCompileRequest {
                        budget,
                        focus_terms,
                    },
                    max_output_tokens,
                    verification: VerificationCommandRequest::new(
                        command[0].clone(),
                        command[1..].to_vec(),
                    ),
                },
            )?;

            print_small_coding_task_result(&result);
            Ok(())
        }
        "recover-fixture" => {
            let session_id = args
                .get(1)
                .ok_or_else(|| HarnessError::new("session id is required"))?;
            let session_id = Uuid::parse_str(session_id)
                .map_err(|error| HarnessError::new(error.to_string()))?;
            let before_separator = args_before_separator(&args);
            let database_url = database_url_from_args(before_separator.to_vec())?;
            let task = required_arg_value(before_separator, "--task")?;
            let budget = context_budget_from_args(before_separator)?;
            let focus_terms = repeated_arg_values(before_separator, "--focus");
            let max_output_tokens = optional_arg_value(before_separator, "--max-output-tokens")
                .map(|value| parse_usize_arg("--max-output-tokens", value))
                .transpose()?
                .unwrap_or(256);
            let max_recovery_rounds = optional_arg_value(before_separator, "--max-recovery-rounds")
                .map(|value| parse_usize_arg("--max-recovery-rounds", value))
                .transpose()?
                .unwrap_or(harness_runtime::SELF_RECOVERY_MAX_ROUNDS);
            let max_repair_bytes = optional_arg_value(before_separator, "--max-repair-bytes")
                .map(|value| parse_usize_arg("--max-repair-bytes", value))
                .transpose()?
                .unwrap_or(16 * 1024);
            let verification = optional_command_after_separator(&args)
                .map(|command| {
                    VerificationCommandRequest::new(command[0].clone(), command[1..].to_vec())
                })
                .unwrap_or_else(|| {
                    VerificationCommandRequest::new(
                        "cargo",
                        vec!["test".to_owned(), "--quiet".to_owned()],
                    )
                });
            let mut runtime = Runtime::connect_postgres(&database_url)?;
            let result = runtime.run_self_recovery_fixture_task(
                session_id,
                SelfRecoveryLoopRequest {
                    task: task.to_owned(),
                    context: SessionContextCompileRequest {
                        budget,
                        focus_terms,
                    },
                    max_output_tokens,
                    verification,
                    max_recovery_rounds,
                    max_repair_bytes,
                },
            )?;

            print_self_recovery_loop_result(&result);
            Ok(())
        }
        "approval" => {
            let action = args
                .get(1)
                .ok_or_else(|| HarnessError::new("approval action is required"))?;
            let session_id = args
                .get(2)
                .ok_or_else(|| HarnessError::new("session id is required"))?;
            let session_id = Uuid::parse_str(session_id)
                .map_err(|error| HarnessError::new(error.to_string()))?;
            let database_url = database_url_from_args(args.clone())?;
            let mut runtime = Runtime::connect_postgres(&database_url)?;
            let approval = match action.as_str() {
                "show" => runtime.show_approval(session_id)?,
                "approve" => runtime.approve_pending_diff(session_id)?,
                "reject" => {
                    let reason = required_arg_value(&args, "--reason")?;
                    runtime.reject_pending_diff(session_id, reason)?
                }
                other => {
                    return Err(HarnessError::new(format!(
                        "unknown approval action: {other}"
                    )));
                }
            };

            print_approval_projection(&approval);
            Ok(())
        }
        "commit" => {
            let session_id = args
                .get(1)
                .ok_or_else(|| HarnessError::new("session id is required"))?;
            let session_id = Uuid::parse_str(session_id)
                .map_err(|error| HarnessError::new(error.to_string()))?;
            let database_url = database_url_from_args(args.clone())?;
            let message = required_arg_value(&args, "--message")?;
            let mut runtime = Runtime::connect_postgres(&database_url)?;
            let commit = runtime.commit_approved_diff(session_id, message)?;

            print_commit_handoff_projection(&commit);
            Ok(())
        }
        "codex-acceptance" => {
            let session_id = args
                .get(1)
                .ok_or_else(|| HarnessError::new("session id is required"))?;
            let session_id = Uuid::parse_str(session_id)
                .map_err(|error| HarnessError::new(error.to_string()))?;
            let before_separator = args_before_separator(&args);
            let database_url = database_url_from_args(before_separator.to_vec())?;
            let task = optional_arg_value(before_separator, "--task")
                .unwrap_or(DEFAULT_CODEX_CLI_ACCEPTANCE_TASK);
            let codex_program = optional_arg_value(before_separator, "--codex-program")
                .unwrap_or(DEFAULT_CODEX_CLI_PROGRAM);
            let timeout_ms = optional_arg_value(before_separator, "--timeout-ms")
                .map(|value| parse_u64_arg("--timeout-ms", value))
                .transpose()?
                .unwrap_or(DEFAULT_CODEX_CLI_ACCEPTANCE_TIMEOUT_MS);
            let max_stdout_bytes = optional_arg_value(before_separator, "--max-stdout-bytes")
                .map(|value| parse_usize_arg("--max-stdout-bytes", value))
                .transpose()?
                .unwrap_or(128 * 1024);
            let codex_home =
                optional_arg_value(before_separator, "--codex-home").map(PathBuf::from);
            let mut request =
                CodexCliAcceptanceRequest::from_env(task.to_owned(), codex_program.to_owned());
            request.timeout_ms = timeout_ms;
            request.max_stdout_bytes = max_stdout_bytes;
            request.availability = CodexCliAvailabilityRequest {
                program: codex_program.to_owned(),
                codex_home: codex_home.or(request.availability.codex_home),
                codex_api_key_present: request.availability.codex_api_key_present,
            };

            let mut runtime = Runtime::connect_postgres(&database_url)?;
            let result = runtime.run_codex_cli_manual_acceptance(session_id, request)?;

            print_codex_cli_acceptance_result(&result);
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

fn repeated_arg_values(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter_map(|window| (window[0] == name).then_some(window[1].clone()))
        .collect()
}

fn context_budget_from_args(args: &[String]) -> HarnessResult<ContextBudget> {
    let mut budget = ContextBudget::default();

    if let Some(value) = optional_arg_value(args, "--max-bytes") {
        budget.max_bytes = parse_usize_arg("--max-bytes", value)?;
    }

    if let Some(value) = optional_arg_value(args, "--max-files") {
        budget.max_files = parse_usize_arg("--max-files", value)?;
    }

    if let Some(value) = optional_arg_value(args, "--max-skill-files") {
        budget.max_skill_files = parse_usize_arg("--max-skill-files", value)?;
    }

    Ok(budget)
}

fn parse_usize_arg(name: &str, value: &str) -> HarnessResult<usize> {
    value
        .parse()
        .map_err(|error| HarnessError::new(format!("{name} must be a positive integer: {error}")))
}

fn parse_u64_arg(name: &str, value: &str) -> HarnessResult<u64> {
    value
        .parse()
        .map_err(|error| HarnessError::new(format!("{name} must be a positive integer: {error}")))
}

fn args_before_separator(args: &[String]) -> &[String] {
    let index = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());

    &args[..index]
}

fn command_after_separator(args: &[String]) -> HarnessResult<&[String]> {
    let Some(index) = args.iter().position(|arg| arg == "--") else {
        return Err(HarnessError::new(
            "verification command must follow -- separator",
        ));
    };

    let command = &args[index + 1..];

    if command.is_empty() {
        return Err(HarnessError::new("verification command is required"));
    }

    Ok(command)
}

fn optional_command_after_separator(args: &[String]) -> Option<&[String]> {
    let index = args.iter().position(|arg| arg == "--")?;
    let command = &args[index + 1..];

    (!command.is_empty()).then_some(command)
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

fn print_task_list(tasks: &[TaskProjection]) {
    println!("task_count={}", tasks.len());
    println!(
        "task_ids={}",
        tasks
            .iter()
            .map(|task| task.task_id.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );

    for (index, task) in tasks.iter().enumerate() {
        println!("task_{index}_id={}", task.task_id);
        println!("task_{index}_status={}", task.status);
        println!("task_{index}_repo_path={}", task.repo_path);
        println!("task_{index}_input={}", task.input);
        println!("task_{index}_worker_lane_kind={}", task.worker_lane_kind);
    }
}

fn print_task_projection(task: &TaskProjection) {
    println!("session_id={}", task.session_id);
    println!("task_id={}", task.task_id);
    println!("task_status={}", task.status);
    println!("task_repo_path={}", task.repo_path);
    println!("task_input={}", task.input);
    println!("task_worker_lane_kind={}", task.worker_lane_kind);
    println!("task_context_max_bytes={}", task.context_budget.max_bytes);
    println!("task_context_max_files={}", task.context_budget.max_files);
    println!(
        "task_context_max_skill_files={}",
        task.context_budget.max_skill_files
    );
    println!("task_focus_terms={}", task.focus_terms.join(","));
    println!("task_max_output_tokens={}", task.max_output_tokens);

    if let Some(worktree) = &task.worktree {
        println!("task_worktree_lane_id={}", worktree.lane_id);
        println!("task_worktree_lane_kind={}", worktree.lane_kind);
        println!("task_worktree_path={}", worktree.worktree_path);
        println!("task_worktree_base_ref={}", worktree.base_ref);
    } else {
        println!("task_worktree_lane_id=");
        println!("task_worktree_lane_kind=");
        println!("task_worktree_path=");
        println!("task_worktree_base_ref=");
    }

    if let Some(worker_output) = &task.worker_output {
        println!("task_worker_lane_id={}", worker_output.lane_id);
        println!("task_worker_lane_kind={}", worker_output.lane_kind);
        println!("task_worker_status={}", worker_output.status);
        println!(
            "task_worker_exit_code={}",
            worker_output
                .exit_code
                .map_or_else(String::new, |exit_code| exit_code.to_string())
        );
        println!("task_worker_stdout={}", worker_output.stdout);
        println!("task_worker_stderr={}", worker_output.stderr);
        println!("task_worker_duration_ms={}", worker_output.duration_ms);
    } else {
        println!("task_worker_lane_id=");
        println!("task_worker_lane_kind=");
        println!("task_worker_status=");
        println!("task_worker_exit_code=");
        println!("task_worker_stdout=");
        println!("task_worker_stderr=");
        println!("task_worker_duration_ms=");
    }

    if let Some(diff) = &task.diff {
        println!("task_diff_files_changed={}", diff.files_changed);
        println!("task_diff_insertions={}", diff.insertions);
        println!("task_diff_deletions={}", diff.deletions);
        println!("task_diff_paths={}", diff.paths.join(","));
    } else {
        println!("task_diff_files_changed=");
        println!("task_diff_insertions=");
        println!("task_diff_deletions=");
        println!("task_diff_paths=");
    }

    if let Some(approval) = &task.approval {
        println!("task_approval_state={}", approval.state);
        println!("task_approval_summary={}", approval.summary);
        println!(
            "task_approval_rejection_reason={}",
            approval.rejection_reason.as_deref().unwrap_or("")
        );
    } else {
        println!("task_approval_state=");
        println!("task_approval_summary=");
        println!("task_approval_rejection_reason=");
    }

    if let Some(commit) = &task.commit {
        println!("task_commit_state={}", commit.state);
        println!("task_commit_message={}", commit.message);
        println!(
            "task_commit_sha={}",
            commit.commit_sha.as_deref().unwrap_or("")
        );
        println!(
            "task_commit_failure_reason={}",
            commit.failure_reason.as_deref().unwrap_or("")
        );
    } else {
        println!("task_commit_state=");
        println!("task_commit_message=");
        println!("task_commit_sha=");
        println!("task_commit_failure_reason=");
    }

    println!("event_count={}", task.event_count);
}

fn print_verification_result(result: &VerificationCommandResult) {
    println!("session_id={}", result.session_id);
    println!("policy_decision={}", result.decision.as_str());
    println!("policy_reason={}", result.reason);
    println!("tool_executed={}", result.observation.is_some());

    if let Some(observation) = &result.observation {
        if let Some(exit_code) = observation.exit_code {
            println!("exit_code={exit_code}");
        } else {
            println!("exit_code=signal");
        }

        println!("duration_ms={}", observation.duration_ms);
    }

    println!("event_count={}", result.event_count);
}

fn print_context_compile_result(result: &SessionContextCompileResult) {
    println!("session_id={}", result.session_id);
    println!("context_sources={}", result.bundle.sources.len());
    println!("context_skills={}", result.bundle.skills.len());
    println!("context_used_bytes={}", result.bundle.used_bytes);
    println!("context_truncated={}", result.bundle.truncated);
    println!("event_count={}", result.event_count);
}

fn print_fake_model_turn_result(result: &FakeModelTurnResult) {
    println!("session_id={}", result.session_id);
    println!("patch_path={}", result.patch.path);
    println!("policy_decision={}", result.decision.as_str());
    println!("policy_reason={}", result.reason);
    println!("patch_applied={}", result.observation.is_some());
    println!("prompt_tokens={}", result.prompt_tokens);
    println!("completion_tokens={}", result.completion_tokens);
    println!("event_count={}", result.event_count);
}

fn print_small_coding_task_result(result: &SmallCodingTaskResult) {
    println!("session_id={}", result.session_id);
    println!("patch_path={}", result.patch.path);
    println!("patch_applied={}", result.patch_applied);
    println!(
        "verification_decision={}",
        result.verification.decision.as_str()
    );
    println!(
        "verification_executed={}",
        result.verification.observation.is_some()
    );

    if let Some(observation) = &result.verification.observation {
        if let Some(exit_code) = observation.exit_code {
            println!("verification_exit_code={exit_code}");
        } else {
            println!("verification_exit_code=signal");
        }
    }

    println!("diff_files_changed={}", result.diff.files_changed);
    println!("diff_insertions={}", result.diff.insertions);
    println!("diff_deletions={}", result.diff.deletions);
    println!("diff_paths={}", result.diff.paths.join(","));
    println!("event_replay_total={}", result.event_replay.total_events);
    println!(
        "event_replay_last={}",
        result.event_replay.last_event_type.as_deref().unwrap_or("")
    );
    println!("token_prompt={}", result.token_ledger.prompt_tokens);
    println!("token_completion={}", result.token_ledger.completion_tokens);
    println!("token_total={}", result.token_ledger.total_tokens);
    println!("token_max_output={}", result.token_ledger.max_output_tokens);
    println!("final_state={}", result.final_state);
    println!("event_count={}", result.event_count);
}

fn print_self_recovery_loop_result(result: &SelfRecoveryLoopResult) {
    println!("session_id={}", result.session_id);
    println!(
        "recovery_classification={}",
        result
            .report
            .failure_classification
            .as_deref()
            .unwrap_or("")
    );
    println!(
        "recovery_plan={}",
        result.report.recovery_plan.as_deref().unwrap_or("")
    );
    println!("recovery_attempts={}", result.report.repair_attempts);
    println!("recovery_retries={}", result.report.retry_count);
    println!(
        "recovery_stop_reason={}",
        result.report.stop_reason.as_str()
    );
    println!("recovery_max_rounds={}", result.report.max_recovery_rounds);
    println!(
        "recovery_used_repair_bytes={}",
        result.report.used_repair_bytes
    );
    println!(
        "verification_decision={}",
        result.final_verification.decision.as_str()
    );
    println!(
        "verification_executed={}",
        result.final_verification.observation.is_some()
    );

    if let Some(observation) = &result.final_verification.observation {
        if let Some(exit_code) = observation.exit_code {
            println!("verification_exit_code={exit_code}");
        } else {
            println!("verification_exit_code=signal");
        }
    }

    println!("diff_files_changed={}", result.diff.files_changed);
    println!("diff_insertions={}", result.diff.insertions);
    println!("diff_deletions={}", result.diff.deletions);
    println!("diff_paths={}", result.diff.paths.join(","));
    println!("event_replay_total={}", result.event_replay.total_events);
    println!(
        "event_replay_last={}",
        result.event_replay.last_event_type.as_deref().unwrap_or("")
    );
    println!("token_prompt={}", result.token_ledger.prompt_tokens);
    println!("token_completion={}", result.token_ledger.completion_tokens);
    println!("token_total={}", result.token_ledger.total_tokens);
    println!("token_max_output={}", result.token_ledger.max_output_tokens);
    println!("final_state={}", result.final_state);
    println!("event_count={}", result.event_count);
}

fn print_approval_projection(approval: &ApprovalProjection) {
    println!("session_id={}", approval.session_id);
    println!("approval_state={}", approval.state);
    println!("approval_summary={}", approval.summary);
    println!(
        "approval_rejection_reason={}",
        approval.rejection_reason.as_deref().unwrap_or("")
    );
    println!("diff_files_changed={}", approval.diff.files_changed);
    println!("diff_insertions={}", approval.diff.insertions);
    println!("diff_deletions={}", approval.diff.deletions);
    println!("diff_paths={}", approval.diff.paths.join(","));
    println!("event_count={}", approval.event_count);
}

fn print_commit_handoff_projection(commit: &CommitHandoffProjection) {
    println!("session_id={}", commit.session_id);
    println!("commit_state={}", commit.state);
    println!("commit_repo_path={}", commit.repo_path);
    println!("commit_message={}", commit.message);
    println!("commit_sha={}", commit.commit_sha.as_deref().unwrap_or(""));
    println!(
        "commit_failure_reason={}",
        commit.failure_reason.as_deref().unwrap_or("")
    );
    println!("event_count={}", commit.event_count);
}

fn print_codex_cli_acceptance_result(result: &CodexCliAcceptanceResult) {
    println!("session_id={}", result.session_id);
    println!("codex_acceptance_status={}", result.status.as_str());
    println!("codex_program={}", result.availability.program);
    println!("codex_available={}", result.availability.available);
    println!("codex_authenticated={}", result.availability.authenticated);
    println!(
        "codex_version={}",
        result.availability.version.as_deref().unwrap_or("")
    );
    println!(
        "codex_skipped_reason={}",
        result.availability.skipped_reason.as_deref().unwrap_or("")
    );

    if let Some(worker) = &result.worker {
        println!("worker_lane_id={}", worker.lane_id);
        println!("worker_final_status={}", worker.final_status.as_str());
        println!(
            "worker_pending_commit_state={}",
            worker.pending_commit_state.as_deref().unwrap_or("")
        );

        if let Some(observation) = &worker.observation {
            if let Some(exit_code) = observation.exit_code {
                println!("worker_exit_code={exit_code}");
            } else {
                println!("worker_exit_code=signal");
            }

            println!("worker_duration_ms={}", observation.duration_ms);
        }

        println!("event_replay_total={}", worker.event_replay.total_events);
        println!(
            "event_replay_last={}",
            worker.event_replay.last_event_type.as_deref().unwrap_or("")
        );
        println!("event_count={}", worker.event_count);
    }
}
