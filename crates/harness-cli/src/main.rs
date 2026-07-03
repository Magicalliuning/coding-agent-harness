use std::env;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_core::{HarnessError, HarnessResult};
use harness_runtime::{
    ApprovalCommitInspectReport, ApprovalCommitInspectRequest, ApprovalProjection,
    CodexCliAcceptanceRequest, CodexCliAcceptanceResult, CodexCliAvailabilityRequest,
    CodexWorkerLaneRequest, CodexWorkerSubprocess, CommitHandoffProjection, ContextBudget,
    CreateTaskRequest, DEFAULT_CODEX_CLI_ACCEPTANCE_TASK, DEFAULT_CODEX_CLI_ACCEPTANCE_TIMEOUT_MS,
    DEFAULT_CODEX_CLI_PROGRAM, DEFAULT_OPENAI_COMPATIBLE_API_KEY_ENV,
    DEFAULT_TASK_WORKER_LANE_KIND, DEFAULT_TIMELINE_PAYLOAD_LIMIT_BYTES,
    EventTimelineInspectRequest, EventTimelineReport, FakeModelTurnRequest, FakeModelTurnResult,
    HeartbeatTaskLeaseRequest, LeaseNextTaskRequest, LeasedCodexWorkerTaskRequest,
    LeasedCodexWorkerTaskResult, ModelProviderRoutingRequest, ModelProviderRoutingResult, Runtime,
    RuntimeArchitectureReport, RuntimeDataFlowReport, RuntimeDataFlowStep, RuntimeModelProvider,
    RuntimeReportEdge, RuntimeReportNode, RuntimeReportNonGoal, RuntimeReportSection,
    RuntimeStorageProjectionReport, RuntimeStorageReport, RuntimeStorageTableReport,
    SelfRecoveryLoopRequest, SelfRecoveryLoopResult, SessionContextCompileRequest,
    SessionContextCompileResult, SessionInspectReport, SessionProjection, SmallCodingTaskRequest,
    SmallCodingTaskResult, StartSessionRequest, TaskInspectReport, TaskProjection,
    TaskQueueInspectReport, TaskQueueInspectRequest, TaskQueueInspectStatusCount,
    VerificationCommandRequest, VerificationCommandResult, WorkerLaneDiffInspectReport,
    WorkerLaneDiffInspectRequest,
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
        Some("report") => run_report_command(args.collect()),
        Some("session") => run_session_command(args.collect()),
        Some(other) => Err(HarnessError::new(format!(
            "unknown command: {other}; usage: harness-cli [--version|doctor|migrate|report|session]"
        ))),
    }
}

fn run_report_command(args: Vec<String>) -> HarnessResult<()> {
    let Some(command) = args.first() else {
        return Err(HarnessError::new("report command is required"));
    };

    match command.as_str() {
        "architecture" => {
            let report = Runtime::architecture_report();
            if has_flag(&args, "--json") {
                print_architecture_report_json(&report)
            } else {
                print_architecture_report(&report);
                Ok(())
            }
        }
        "data-flow" => {
            let report = Runtime::data_flow_report();
            if has_flag(&args, "--json") {
                print_data_flow_report_json(&report)
            } else {
                print_data_flow_report(&report);
                Ok(())
            }
        }
        "storage" => {
            let report = Runtime::storage_report();
            if has_flag(&args, "--json") {
                print_storage_report_json(&report)
            } else {
                print_storage_report(&report);
                Ok(())
            }
        }
        other => Err(HarnessError::new(format!(
            "unknown report command: {other}; usage: harness-cli report [architecture|data-flow|storage] [--json]"
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
        "inspect" => {
            let session_id = session_id_arg(&args, 1)?;
            let database_url = database_url_from_args(args.clone())?;
            let runtime = Runtime::connect_postgres(&database_url)?;
            let report = runtime.inspect_session(session_id)?;

            if has_flag(&args, "--json") {
                print_session_inspect_json(&report)?;
            } else {
                print_session_inspect_report(&report);
            }

            Ok(())
        }
        "timeline" => {
            let session_id = session_id_arg(&args, 1)?;
            let task_id = optional_arg_value(&args, "--task-id")
                .map(|task_id| {
                    Uuid::parse_str(task_id).map_err(|error| {
                        HarnessError::new(format!("task id must be a UUID: {error}"))
                    })
                })
                .transpose()?;
            let payload_limit_bytes = optional_arg_value(&args, "--payload-bytes")
                .map(|value| parse_usize_arg("--payload-bytes", value))
                .transpose()?
                .unwrap_or(DEFAULT_TIMELINE_PAYLOAD_LIMIT_BYTES);
            let database_url = database_url_from_args(args.clone())?;
            let runtime = Runtime::connect_postgres(&database_url)?;
            let mut request = EventTimelineInspectRequest::new(session_id)
                .with_payload_limit_bytes(payload_limit_bytes);
            if let Some(task_id) = task_id {
                request = request.with_task_id(task_id);
            }
            let report = runtime.inspect_event_timeline(request)?;

            if has_flag(&args, "--json") {
                print_event_timeline_json(&report)?;
            } else {
                print_event_timeline_report(&report);
            }

            Ok(())
        }
        "worker" => {
            let action = args
                .get(1)
                .ok_or_else(|| HarnessError::new("worker action is required"))?;
            if action != "inspect" {
                return Err(HarnessError::new(format!(
                    "unknown worker action: {action}; usage: harness-cli session worker inspect <session_id> [--task-id <task_id>] [--payload-bytes <bytes>] [--json]"
                )));
            }

            let session_id = session_id_arg(&args, 2)?;
            let task_id = optional_arg_value(&args, "--task-id")
                .map(|task_id| {
                    Uuid::parse_str(task_id).map_err(|error| {
                        HarnessError::new(format!("task id must be a UUID: {error}"))
                    })
                })
                .transpose()?;
            let payload_limit_bytes = optional_arg_value(&args, "--payload-bytes")
                .map(|value| parse_usize_arg("--payload-bytes", value))
                .transpose()?
                .unwrap_or(DEFAULT_TIMELINE_PAYLOAD_LIMIT_BYTES);
            let database_url = database_url_from_args(args.clone())?;
            let runtime = Runtime::connect_postgres(&database_url)?;
            let mut request = WorkerLaneDiffInspectRequest::new(session_id)
                .with_payload_limit_bytes(payload_limit_bytes);
            if let Some(task_id) = task_id {
                request = request.with_task_id(task_id);
            }
            let report = runtime.inspect_worker_lane_diff(request)?;

            if has_flag(&args, "--json") {
                print_worker_lane_diff_inspect_json(&report)?;
            } else {
                print_worker_lane_diff_inspect_report(&report);
            }

            Ok(())
        }
        "task" => {
            let action = args
                .get(1)
                .ok_or_else(|| HarnessError::new("task action is required"))?;
            let database_args = if action == "run-next-codex-worker" {
                args_before_separator(&args).to_vec()
            } else {
                args.clone()
            };
            let database_url = database_url_from_args(database_args)?;
            let mut runtime = Runtime::connect_postgres(&database_url)?;

            match action.as_str() {
                "create" => {
                    let session_id = session_id_arg(&args, 2)?;
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
                    let session_id = session_id_arg(&args, 2)?;
                    let tasks = runtime.list_tasks(session_id)?;
                    print_task_list(&tasks);
                    Ok(())
                }
                "show" => {
                    let session_id = session_id_arg(&args, 2)?;
                    let task_id = args
                        .get(3)
                        .ok_or_else(|| HarnessError::new("task id is required"))?;
                    let task_id = Uuid::parse_str(task_id)
                        .map_err(|error| HarnessError::new(error.to_string()))?;
                    let task = runtime.show_task(session_id, task_id)?;
                    print_task_projection(&task);
                    Ok(())
                }
                "inspect" => {
                    let session_id = session_id_arg(&args, 2)?;
                    let task_id = task_id_arg(&args, 3)?;
                    let report = runtime.inspect_task(session_id, task_id)?;

                    if has_flag(&args, "--json") {
                        print_task_inspect_json(&report)?;
                    } else {
                        print_task_inspect_report(&report);
                    }

                    Ok(())
                }
                "queue-inspect" => {
                    let session_id = optional_arg_value(&args, "--session-id")
                        .map(|session_id| {
                            Uuid::parse_str(session_id).map_err(|error| {
                                HarnessError::new(format!("session id must be a UUID: {error}"))
                            })
                        })
                        .transpose()?;
                    let now_ms = optional_arg_value(&args, "--now-ms")
                        .map(|value| parse_i64_arg("--now-ms", value))
                        .transpose()?;
                    let mut request = TaskQueueInspectRequest::new();
                    if let Some(session_id) = session_id {
                        request = request.with_session_id(session_id);
                    }
                    if let Some(now_ms) = now_ms {
                        request = request.with_now_ms(now_ms);
                    }
                    let report = runtime.inspect_task_queue(request)?;

                    if has_flag(&args, "--json") {
                        print_task_queue_inspect_json(&report)?;
                    } else {
                        print_task_queue_inspect_report(&report);
                    }

                    Ok(())
                }
                "enqueue" => {
                    let session_id = session_id_arg(&args, 2)?;
                    let task_id = task_id_arg(&args, 3)?;
                    let max_retries = optional_arg_value(&args, "--max-retries")
                        .map(|value| parse_i32_arg("--max-retries", value))
                        .transpose()?
                        .unwrap_or(1);
                    let task =
                        runtime.enqueue_task_with_max_retries(session_id, task_id, max_retries)?;
                    print_task_projection(&task);
                    Ok(())
                }
                "lease-next" => {
                    let worker_id = required_arg_value(&args, "--worker-id")?;
                    let lease_duration_ms = optional_arg_value(&args, "--lease-ms")
                        .map(|value| parse_u64_arg("--lease-ms", value))
                        .transpose()?
                        .unwrap_or(60_000);
                    let task = runtime.lease_next_task(LeaseNextTaskRequest {
                        worker_id: worker_id.to_owned(),
                        lease_duration_ms,
                    })?;

                    if let Some(task) = task {
                        println!("task_leased=true");
                        print_task_projection(&task);
                    } else {
                        println!("task_leased=false");
                    }

                    Ok(())
                }
                "heartbeat" => {
                    let task_id = task_id_arg(&args, 2)?;
                    let lease_id = lease_id_arg(&args)?;
                    let worker_id = required_arg_value(&args, "--worker-id")?;
                    let lease_duration_ms = optional_arg_value(&args, "--lease-ms")
                        .map(|value| parse_u64_arg("--lease-ms", value))
                        .transpose()?
                        .unwrap_or(60_000);
                    let task = runtime.heartbeat_task_lease(
                        task_id,
                        lease_id,
                        HeartbeatTaskLeaseRequest {
                            worker_id: worker_id.to_owned(),
                            lease_duration_ms,
                        },
                    )?;
                    print_task_projection(&task);
                    Ok(())
                }
                "expire-leases" => {
                    let now_ms = optional_arg_value(&args, "--now-ms")
                        .map(|value| parse_i64_arg("--now-ms", value))
                        .transpose()?
                        .unwrap_or_else(current_time_ms_for_cli);
                    let tasks = runtime.expire_task_leases(now_ms)?;
                    println!("expired_task_count={}", tasks.len());
                    for (index, task) in tasks.iter().enumerate() {
                        println!("expired_task_{index}_id={}", task.task_id);
                        println!("expired_task_{index}_status={}", task.status);
                    }
                    if tasks.len() == 1 {
                        print_task_projection(&tasks[0]);
                    }
                    Ok(())
                }
                "run-next-codex-worker" => {
                    let before_separator = args_before_separator(&args);
                    let worker_id = required_arg_value(before_separator, "--worker-id")?;
                    let lease_duration_ms = optional_arg_value(before_separator, "--lease-ms")
                        .map(|value| parse_u64_arg("--lease-ms", value))
                        .transpose()?
                        .unwrap_or(60_000);
                    let timeout_ms = optional_arg_value(before_separator, "--timeout-ms")
                        .map(|value| parse_u64_arg("--timeout-ms", value))
                        .transpose()?
                        .unwrap_or(30_000);
                    let max_stdout_bytes =
                        optional_arg_value(before_separator, "--max-stdout-bytes")
                            .map(|value| parse_usize_arg("--max-stdout-bytes", value))
                            .transpose()?
                            .unwrap_or(64 * 1024);
                    let command = command_after_separator(&args)?;
                    let mut worker = CodexWorkerLaneRequest::new_subprocess(
                        "queued task",
                        CodexWorkerSubprocess::new(command[0].clone(), command[1..].to_vec()),
                    );
                    worker.timeout_ms = timeout_ms;
                    worker.budget.max_stdout_bytes = max_stdout_bytes;

                    let result = runtime.run_next_leased_codex_worker_task(
                        LeasedCodexWorkerTaskRequest {
                            worker_id: worker_id.to_owned(),
                            lease_duration_ms,
                            worker,
                        },
                    )?;

                    if let Some(result) = result {
                        println!("task_leased=true");
                        print_leased_codex_worker_task_result(&result);
                    } else {
                        println!("task_leased=false");
                    }

                    Ok(())
                }
                "approve" => {
                    let session_id = session_id_arg(&args, 2)?;
                    let task_id = task_id_arg(&args, 3)?;
                    let task = runtime.approve_task_pending_diff(session_id, task_id)?;
                    print_task_projection(&task);
                    Ok(())
                }
                "reject" => {
                    let session_id = session_id_arg(&args, 2)?;
                    let task_id = task_id_arg(&args, 3)?;
                    let reason = required_arg_value(&args, "--reason")?;
                    let task = runtime.reject_task_pending_diff(session_id, task_id, reason)?;
                    print_task_projection(&task);
                    Ok(())
                }
                "commit" => {
                    let session_id = session_id_arg(&args, 2)?;
                    let task_id = task_id_arg(&args, 3)?;
                    let message = required_arg_value(&args, "--message")?;
                    let task = runtime.commit_approved_task_diff(session_id, task_id, message)?;
                    print_task_projection(&task);
                    Ok(())
                }
                "complete" => {
                    let task_id = task_id_arg(&args, 2)?;
                    let lease_id = lease_id_arg(&args)?;
                    let reason = required_arg_value(&args, "--reason")?;
                    let task = runtime.complete_task_lease(task_id, lease_id, reason)?;
                    print_task_projection(&task);
                    Ok(())
                }
                "fail" => {
                    let task_id = task_id_arg(&args, 2)?;
                    let lease_id = lease_id_arg(&args)?;
                    let reason = required_arg_value(&args, "--reason")?;
                    let task = runtime.fail_task_lease(task_id, lease_id, reason)?;
                    print_task_projection(&task);
                    Ok(())
                }
                "cancel" => {
                    let task_id = task_id_arg(&args, 2)?;
                    let lease_id = lease_id_arg(&args)?;
                    let reason = required_arg_value(&args, "--reason")?;
                    let task = runtime.cancel_task_lease(task_id, lease_id, reason)?;
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
        "model-route" => {
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
            let provider = model_provider_from_args(&args)?;
            let mut runtime = Runtime::connect_postgres(&database_url)?;
            let result = runtime.run_model_provider_route(
                session_id,
                ModelProviderRoutingRequest {
                    provider,
                    task: task.to_owned(),
                    context: SessionContextCompileRequest {
                        budget,
                        focus_terms,
                    },
                    max_output_tokens,
                },
            )?;

            print_model_provider_routing_result(&result);
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
            if action == "inspect" {
                let task_id = optional_arg_value(&args, "--task-id")
                    .map(|task_id| {
                        Uuid::parse_str(task_id).map_err(|error| {
                            HarnessError::new(format!("task id must be a UUID: {error}"))
                        })
                    })
                    .transpose()?;
                let mut request = ApprovalCommitInspectRequest::new(session_id);
                if let Some(task_id) = task_id {
                    request = request.with_task_id(task_id);
                }
                let report = runtime.inspect_approval_commit(request)?;

                if has_flag(&args, "--json") {
                    print_approval_commit_inspect_json(&report)?;
                } else {
                    print_approval_commit_inspect_report(&report);
                }

                return Ok(());
            }

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

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
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

fn model_provider_from_args(args: &[String]) -> HarnessResult<RuntimeModelProvider> {
    match optional_arg_value(args, "--provider").unwrap_or("deterministic-fake") {
        "deterministic-fake" | "fake" => Ok(RuntimeModelProvider::deterministic_fake()),
        "openai-compatible" => {
            let model_id = required_arg_value(args, "--model")?;
            let api_key_env = optional_arg_value(args, "--api-key-env")
                .unwrap_or(DEFAULT_OPENAI_COMPATIBLE_API_KEY_ENV);

            Ok(RuntimeModelProvider::OpenAiCompatible {
                model_id: model_id.to_owned(),
                api_key_env: api_key_env.to_owned(),
            })
        }
        other => Err(HarnessError::new(format!(
            "unknown model provider: {other}; expected deterministic-fake or openai-compatible"
        ))),
    }
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

fn parse_i32_arg(name: &str, value: &str) -> HarnessResult<i32> {
    let parsed = value
        .parse()
        .map_err(|error| HarnessError::new(format!("{name} must be an integer: {error}")))?;

    if parsed < 0 {
        return Err(HarnessError::new(format!("{name} cannot be negative")));
    }

    Ok(parsed)
}

fn parse_i64_arg(name: &str, value: &str) -> HarnessResult<i64> {
    value
        .parse()
        .map_err(|error| HarnessError::new(format!("{name} must be an integer: {error}")))
}

fn current_time_ms_for_cli() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
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

fn session_id_arg(args: &[String], index: usize) -> HarnessResult<Uuid> {
    let session_id = args
        .get(index)
        .ok_or_else(|| HarnessError::new("session id is required"))?;
    Uuid::parse_str(session_id)
        .map_err(|error| HarnessError::new(format!("session id must be a UUID: {error}")))
}

fn task_id_arg(args: &[String], index: usize) -> HarnessResult<Uuid> {
    let task_id = args
        .get(index)
        .ok_or_else(|| HarnessError::new("task id is required"))?;
    Uuid::parse_str(task_id).map_err(|error| HarnessError::new(error.to_string()))
}

fn lease_id_arg(args: &[String]) -> HarnessResult<Uuid> {
    let lease_id = required_arg_value(args, "--lease-id")?;
    Uuid::parse_str(lease_id).map_err(|error| HarnessError::new(error.to_string()))
}

fn print_projection(projection: &SessionProjection) {
    println!("session_id={}", projection.session_id);
    println!("status={}", projection.status.as_str());
    println!("repo_path={}", projection.repo_path);
    println!("event_count={}", projection.event_count);
}

fn print_architecture_report(report: &RuntimeArchitectureReport) {
    print_report_header(
        &report.report_id,
        &report.title,
        &report.summary,
        &report.boundary_adrs,
        &report.source_of_truth,
        &report.projection_kind,
    );
    print_report_sections(&report.sections);
    print_report_nodes(&report.nodes);
    print_report_edges(&report.edges);
    print_report_non_goals(&report.non_goals);
}

fn print_architecture_report_json(report: &RuntimeArchitectureReport) -> HarnessResult<()> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| HarnessError::new(error.to_string()))?;
    println!("{json}");
    Ok(())
}

fn print_data_flow_report(report: &RuntimeDataFlowReport) {
    print_report_header(
        &report.report_id,
        &report.title,
        &report.summary,
        &report.boundary_adrs,
        &report.source_of_truth,
        &report.projection_kind,
    );
    println!("step_count={}", report.steps.len());
    for (index, step) in report.steps.iter().enumerate() {
        print_data_flow_step(index, step);
    }
    print_report_edges(&report.edges);
    print_report_non_goals(&report.non_goals);
}

fn print_data_flow_report_json(report: &RuntimeDataFlowReport) -> HarnessResult<()> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| HarnessError::new(error.to_string()))?;
    println!("{json}");
    Ok(())
}

fn print_storage_report(report: &RuntimeStorageReport) {
    print_report_header(
        &report.report_id,
        &report.title,
        &report.summary,
        &report.boundary_adrs,
        &report.source_of_truth,
        &report.projection_kind,
    );
    print_report_sections(&report.sections);
    println!("table_count={}", report.tables.len());
    for (index, table) in report.tables.iter().enumerate() {
        print_storage_table(index, table);
    }
    println!("projection_count={}", report.projections.len());
    for (index, projection) in report.projections.iter().enumerate() {
        print_storage_projection(index, projection);
    }
    print_report_non_goals(&report.non_goals);
}

fn print_storage_report_json(report: &RuntimeStorageReport) -> HarnessResult<()> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| HarnessError::new(error.to_string()))?;
    println!("{json}");
    Ok(())
}

fn print_report_header(
    report_id: &str,
    title: &str,
    summary: &str,
    boundary_adrs: &[String],
    source_of_truth: &str,
    projection_kind: &str,
) {
    println!("report_id={report_id}");
    println!("title={title}");
    println!("summary={summary}");
    println!("boundary_adrs={}", boundary_adrs.join(","));
    println!("source_of_truth={source_of_truth}");
    println!("projection_kind={projection_kind}");
}

fn print_report_sections(sections: &[RuntimeReportSection]) {
    println!("section_count={}", sections.len());
    for (index, section) in sections.iter().enumerate() {
        println!("section_{index}_id={}", section.id);
        println!("section_{index}_title={}", section.title);
        println!("section_{index}_summary={}", section.summary);
        println!(
            "section_{index}_key_points={}",
            section.key_points.join("|")
        );
    }
}

fn print_report_nodes(nodes: &[RuntimeReportNode]) {
    println!("node_count={}", nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        println!("node_{index}_id={}", node.id);
        println!("node_{index}_label={}", node.label);
        println!("node_{index}_kind={}", node.kind);
        println!("node_{index}_summary={}", node.summary);
        println!(
            "node_{index}_glossary_terms={}",
            node.glossary_terms.join("|")
        );
        println!("node_{index}_adr_refs={}", node.adr_refs.join(","));
    }
}

fn print_report_edges(edges: &[RuntimeReportEdge]) {
    println!("edge_count={}", edges.len());
    for (index, edge) in edges.iter().enumerate() {
        println!("edge_{index}_id={}", edge.id);
        println!("edge_{index}_from={}", edge.from);
        println!("edge_{index}_to={}", edge.to);
        println!("edge_{index}_label={}", edge.label);
        println!("edge_{index}_summary={}", edge.summary);
    }
}

fn print_report_non_goals(non_goals: &[RuntimeReportNonGoal]) {
    println!("non_goal_count={}", non_goals.len());
    for (index, non_goal) in non_goals.iter().enumerate() {
        println!("non_goal_{index}_id={}", non_goal.id);
        println!("non_goal_{index}_summary={}", non_goal.summary);
    }
}

fn print_data_flow_step(index: usize, step: &RuntimeDataFlowStep) {
    println!("step_{index}_id={}", step.id);
    println!("step_{index}_order={}", step.order);
    println!("step_{index}_label={}", step.label);
    println!("step_{index}_component_id={}", step.component_id);
    println!("step_{index}_event_types={}", step.event_types.join(","));
    println!("step_{index}_summary={}", step.summary);
}

fn print_storage_table(index: usize, table: &RuntimeStorageTableReport) {
    println!("table_{index}_id={}", table.id);
    println!("table_{index}_name={}", table.table_name);
    println!("table_{index}_role={}", table.role);
    println!(
        "table_{index}_ownership_boundary={}",
        table.ownership_boundary
    );
    println!("table_{index}_key_fields={}", table.key_fields.join(","));
    println!(
        "table_{index}_derived_from={}",
        table.derived_from.as_deref().unwrap_or_default()
    );
}

fn print_storage_projection(index: usize, projection: &RuntimeStorageProjectionReport) {
    println!("projection_{index}_id={}", projection.id);
    println!("projection_{index}_name={}", projection.name);
    println!("projection_{index}_source={}", projection.source);
    println!("projection_{index}_summary={}", projection.summary);
}

fn print_worker_lane_diff_inspect_report(report: &WorkerLaneDiffInspectReport) {
    println!("session_id={}", report.session_id);
    println!(
        "task_id={}",
        report
            .task_id
            .map_or_else(String::new, |task_id| task_id.to_string())
    );
    println!("worker_lane_scope={}", report.scope);
    println!("lane_count={}", report.lane_count);
    println!("diff_count={}", report.diff_count);
    println!("event_count={}", report.event_count);
    println!("payload_limit_bytes={}", report.payload_limit_bytes);
    println!("source_of_truth={}", report.source_of_truth);
    println!("projection_kind={}", report.projection_kind);

    for (index, lane) in report.lanes.iter().enumerate() {
        println!(
            "lane_{index}_task_id={}",
            lane.task_id
                .map_or_else(String::new, |task_id| task_id.to_string())
        );
        println!("lane_{index}_id={}", lane.lane_id);
        println!("lane_{index}_kind={}", lane.lane_kind);
        println!(
            "lane_{index}_request_status={}",
            lane.request_status.as_deref().unwrap_or_default()
        );
        println!(
            "lane_{index}_policy_tool={}",
            lane.policy
                .as_ref()
                .map(|policy| policy.tool_name.as_str())
                .unwrap_or_default()
        );
        println!(
            "lane_{index}_policy_decision={}",
            lane.policy
                .as_ref()
                .map(|policy| policy.decision.as_str())
                .unwrap_or_default()
        );
        println!(
            "lane_{index}_policy_reason={}",
            lane.policy
                .as_ref()
                .map(|policy| policy.reason.as_str())
                .unwrap_or_default()
        );
        println!(
            "lane_{index}_current_state={}",
            lane.current_state.as_deref().unwrap_or_default()
        );
        println!("lane_{index}_running={}", lane.is_running);
        println!("lane_{index}_terminal={}", lane.is_terminal);
        println!(
            "lane_{index}_terminal_state={}",
            lane.terminal_state.as_deref().unwrap_or_default()
        );
        println!(
            "lane_{index}_terminal_reason={}",
            lane.terminal_reason.as_deref().unwrap_or_default()
        );
        println!(
            "lane_{index}_failure_reason={}",
            lane.failure_reason.as_deref().unwrap_or_default()
        );
        println!(
            "lane_{index}_timeout_reason={}",
            lane.timeout_reason.as_deref().unwrap_or_default()
        );
        println!(
            "lane_{index}_cancellation_reason={}",
            lane.cancellation_reason.as_deref().unwrap_or_default()
        );
        println!(
            "lane_{index}_requested_workspace_path={}",
            lane.workspace
                .requested_workspace_path
                .as_deref()
                .unwrap_or_default()
        );
        println!(
            "lane_{index}_requested_worktree_path={}",
            lane.workspace
                .requested_worktree_path
                .as_deref()
                .unwrap_or_default()
        );
        println!(
            "lane_{index}_allocated_worktree_path={}",
            lane.workspace
                .allocated_worktree_path
                .as_deref()
                .unwrap_or_default()
        );
        println!(
            "lane_{index}_session_repo_path={}",
            lane.workspace
                .session_repo_path
                .as_deref()
                .unwrap_or_default()
        );
        println!(
            "lane_{index}_task_repo_path={}",
            lane.workspace.task_repo_path.as_deref().unwrap_or_default()
        );
        println!(
            "lane_{index}_diff_repo_path={}",
            lane.workspace.diff_repo_path.as_deref().unwrap_or_default()
        );
        println!(
            "lane_{index}_base_ref={}",
            lane.workspace.base_ref.as_deref().unwrap_or_default()
        );
        println!("lane_{index}_state_count={}", lane.states.len());

        if let Some(request) = &lane.request {
            println!("lane_{index}_request_timeout_ms={}", request.timeout_ms);
            println!(
                "lane_{index}_request_cancellation_requested={}",
                request.cancellation_requested
            );
            println!(
                "lane_{index}_request_workspace_path={}",
                request.workspace_path
            );
            println!(
                "lane_{index}_request_worktree_path={}",
                request.worktree_path.as_deref().unwrap_or_default()
            );
            println!(
                "lane_{index}_request_task_original_bytes={}",
                request.task.original_bytes
            );
            println!(
                "lane_{index}_request_task_truncated={}",
                request.task.truncated
            );
            println!("lane_{index}_request_task_summary={}", request.task.text);
            println!(
                "lane_{index}_budget_max_prompt_tokens={}",
                request.budget.max_prompt_tokens
            );
            println!(
                "lane_{index}_budget_max_output_tokens={}",
                request.budget.max_output_tokens
            );
            println!(
                "lane_{index}_budget_max_stdout_bytes={}",
                request.budget.max_stdout_bytes
            );
            println!(
                "lane_{index}_request_event_sequence={}",
                request.event_sequence
            );
        }

        if let Some(observation) = &lane.observation {
            println!("lane_{index}_observation_status={}", observation.status);
            println!(
                "lane_{index}_observation_exit_code={}",
                observation
                    .exit_code
                    .map_or_else(String::new, |exit_code| exit_code.to_string())
            );
            println!(
                "lane_{index}_observation_duration_ms={}",
                observation.duration_ms
            );
            println!(
                "lane_{index}_observation_prompt_tokens={}",
                observation
                    .prompt_tokens
                    .map_or_else(String::new, |tokens| tokens.to_string())
            );
            println!(
                "lane_{index}_observation_completion_tokens={}",
                observation
                    .completion_tokens
                    .map_or_else(String::new, |tokens| tokens.to_string())
            );
            println!(
                "lane_{index}_observation_usage_confidence={}",
                observation.usage_confidence
            );
            println!(
                "lane_{index}_observation_skipped_or_unavailable_reason={}",
                observation
                    .skipped_or_unavailable_reason
                    .as_deref()
                    .unwrap_or_default()
            );
            println!(
                "lane_{index}_stdout_original_bytes={}",
                observation.stdout.original_bytes
            );
            println!(
                "lane_{index}_stdout_truncated={}",
                observation.stdout.truncated
            );
            println!("lane_{index}_stdout_summary={}", observation.stdout.text);
            println!(
                "lane_{index}_stderr_original_bytes={}",
                observation.stderr.original_bytes
            );
            println!(
                "lane_{index}_stderr_truncated={}",
                observation.stderr.truncated
            );
            println!("lane_{index}_stderr_summary={}", observation.stderr.text);
            println!(
                "lane_{index}_observation_event_sequence={}",
                observation.event_sequence
            );
        }
    }

    for (index, diff) in report.diffs.iter().enumerate() {
        println!(
            "diff_{index}_task_id={}",
            diff.task_id
                .map_or_else(String::new, |task_id| task_id.to_string())
        );
        println!(
            "diff_{index}_repo_path={}",
            diff.repo_path.as_deref().unwrap_or_default()
        );
        println!("diff_{index}_files_changed={}", diff.files_changed);
        println!("diff_{index}_insertions={}", diff.insertions);
        println!("diff_{index}_deletions={}", diff.deletions);
        println!("diff_{index}_paths={}", diff.paths.join(","));
        println!(
            "diff_{index}_git_status_original_bytes={}",
            diff.git_status.original_bytes
        );
        println!(
            "diff_{index}_git_status_truncated={}",
            diff.git_status.truncated
        );
        println!("diff_{index}_git_status_summary={}", diff.git_status.text);
        println!(
            "diff_{index}_git_diff_original_bytes={}",
            diff.git_diff.original_bytes
        );
        println!(
            "diff_{index}_git_diff_truncated={}",
            diff.git_diff.truncated
        );
        println!("diff_{index}_git_diff_summary={}", diff.git_diff.text);
        println!("diff_{index}_event_sequence={}", diff.event_sequence);
    }
}

fn print_worker_lane_diff_inspect_json(report: &WorkerLaneDiffInspectReport) -> HarnessResult<()> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| HarnessError::new(error.to_string()))?;
    println!("{json}");
    Ok(())
}

fn print_approval_commit_inspect_report(report: &ApprovalCommitInspectReport) {
    println!("session_id={}", report.session_id);
    println!(
        "task_id={}",
        report
            .task_id
            .map_or_else(String::new, |task_id| task_id.to_string())
    );
    println!("approval_commit_scope={}", report.scope);
    println!("pending_approval_count={}", report.pending_count);
    println!("approval_decision_count={}", report.decision_count);
    println!("commit_handoff_count={}", report.commit_count);
    println!("event_count={}", report.event_count);
    println!("source_of_truth={}", report.source_of_truth);
    println!("projection_kind={}", report.projection_kind);

    for (index, pending) in report.pending_approvals.iter().enumerate() {
        println!(
            "pending_{index}_task_id={}",
            pending
                .task_id
                .map_or_else(String::new, |task_id| task_id.to_string())
        );
        println!("pending_{index}_summary={}", pending.summary);
        println!("pending_{index}_event_sequence={}", pending.event_sequence);
        if let Some(diff) = &pending.diff {
            println!(
                "pending_{index}_diff_repo_path={}",
                diff.repo_path.as_deref().unwrap_or_default()
            );
            println!("pending_{index}_diff_files_changed={}", diff.files_changed);
            println!("pending_{index}_diff_insertions={}", diff.insertions);
            println!("pending_{index}_diff_deletions={}", diff.deletions);
            println!("pending_{index}_diff_paths={}", diff.paths.join(","));
        } else {
            println!("pending_{index}_diff_repo_path=");
            println!("pending_{index}_diff_files_changed=");
            println!("pending_{index}_diff_insertions=");
            println!("pending_{index}_diff_deletions=");
            println!("pending_{index}_diff_paths=");
        }
        println!(
            "pending_{index}_worker_workspace_path={}",
            pending
                .workspace
                .worker_workspace_path
                .as_deref()
                .unwrap_or_default()
        );
        println!(
            "pending_{index}_worker_worktree_path={}",
            pending
                .workspace
                .worker_worktree_path
                .as_deref()
                .unwrap_or_default()
        );
        println!(
            "pending_{index}_allocated_worktree_path={}",
            pending
                .workspace
                .allocated_worktree_path
                .as_deref()
                .unwrap_or_default()
        );
        println!(
            "pending_{index}_session_repo_path={}",
            pending
                .workspace
                .session_repo_path
                .as_deref()
                .unwrap_or_default()
        );
        println!(
            "pending_{index}_workspace_diff_repo_path={}",
            pending
                .workspace
                .diff_repo_path
                .as_deref()
                .unwrap_or_default()
        );
    }

    for (index, decision) in report.decisions.iter().enumerate() {
        println!(
            "decision_{index}_task_id={}",
            decision
                .task_id
                .map_or_else(String::new, |task_id| task_id.to_string())
        );
        println!("decision_{index}_state={}", decision.state);
        println!(
            "decision_{index}_actor={}",
            decision.actor.as_deref().unwrap_or_default()
        );
        println!("decision_{index}_reason={}", decision.reason);
        println!(
            "decision_{index}_rejection_reason={}",
            decision.rejection_reason.as_deref().unwrap_or_default()
        );
        println!(
            "decision_{index}_event_sequence={}",
            decision.event_sequence
        );
    }

    for (index, commit) in report.commits.iter().enumerate() {
        println!(
            "commit_{index}_task_id={}",
            commit
                .task_id
                .map_or_else(String::new, |task_id| task_id.to_string())
        );
        println!("commit_{index}_state={}", commit.state);
        println!(
            "commit_{index}_actor={}",
            commit.actor.as_deref().unwrap_or_default()
        );
        println!("commit_{index}_repo_path={}", commit.repo_path);
        println!("commit_{index}_message={}", commit.message);
        println!(
            "commit_{index}_sha={}",
            commit.commit_sha.as_deref().unwrap_or_default()
        );
        println!(
            "commit_{index}_failure_reason={}",
            commit.failure_reason.as_deref().unwrap_or_default()
        );
        println!("commit_{index}_event_sequence={}", commit.event_sequence);
    }
}

fn print_approval_commit_inspect_json(report: &ApprovalCommitInspectReport) -> HarnessResult<()> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| HarnessError::new(error.to_string()))?;
    println!("{json}");
    Ok(())
}

fn print_session_inspect_report(report: &SessionInspectReport) {
    println!("session_id={}", report.session_id);
    println!("session_status={}", report.status);
    println!("session_repo_path={}", report.repo_path);
    println!("event_count={}", report.event_count);
    println!(
        "latest_event_sequence={}",
        report
            .latest_event_sequence
            .map_or_else(String::new, |sequence| sequence.to_string())
    );
    println!(
        "latest_event_type={}",
        report.latest_event_type.as_deref().unwrap_or_default()
    );
    println!("task_count={}", report.task_count);
    println!(
        "task_status_counts={}",
        report
            .task_status_counts
            .iter()
            .map(|count| format!("{}:{}", count.status, count.count))
            .collect::<Vec<_>>()
            .join(",")
    );
    println!("source_of_truth={}", report.source_of_truth);
    println!("projection_kind={}", report.projection_kind);
}

fn print_session_inspect_json(report: &SessionInspectReport) -> HarnessResult<()> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| HarnessError::new(error.to_string()))?;
    println!("{json}");
    Ok(())
}

fn print_event_timeline_report(report: &EventTimelineReport) {
    println!("session_id={}", report.session_id);
    println!(
        "task_id={}",
        report
            .task_id
            .map_or_else(String::new, |task_id| task_id.to_string())
    );
    println!(
        "timeline_scope={}",
        if report.task_id.is_some() {
            "task"
        } else {
            "session"
        }
    );
    println!("event_count={}", report.event_count);
    println!("payload_limit_bytes={}", report.payload_limit_bytes);
    println!("source_of_truth={}", report.source_of_truth);
    println!("projection_kind={}", report.projection_kind);

    for (index, event) in report.events.iter().enumerate() {
        println!("event_{index}_id={}", event.event_id);
        println!("event_{index}_session_id={}", event.session_id);
        println!("event_{index}_sequence={}", event.sequence);
        println!("event_{index}_type={}", event.event_type);
        println!("event_{index}_schema_version={}", event.schema_version);
        println!(
            "event_{index}_task_id={}",
            event
                .task_id
                .map_or_else(String::new, |task_id| task_id.to_string())
        );
        println!(
            "event_{index}_payload_original_bytes={}",
            event.payload.original_bytes
        );
        println!(
            "event_{index}_payload_truncated={}",
            event.payload.truncated
        );
        println!("event_{index}_payload_summary={}", event.payload.text);
    }
}

fn print_event_timeline_json(report: &EventTimelineReport) -> HarnessResult<()> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| HarnessError::new(error.to_string()))?;
    println!("{json}");
    Ok(())
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

    if let Some(queue) = &task.queue {
        println!("task_queue_status={}", queue.status);
        println!(
            "task_queue_reason={}",
            queue.reason.as_deref().unwrap_or("")
        );
        println!(
            "task_queue_retry_count={}",
            queue
                .retry_count
                .map_or_else(String::new, |retry_count| retry_count.to_string())
        );
        println!(
            "task_queue_max_retries={}",
            queue
                .max_retries
                .map_or_else(String::new, |max_retries| max_retries.to_string())
        );
        println!(
            "task_queue_stop_reason={}",
            queue.stop_reason.as_deref().unwrap_or("")
        );
    } else {
        println!("task_queue_status=");
        println!("task_queue_reason=");
        println!("task_queue_retry_count=");
        println!("task_queue_max_retries=");
        println!("task_queue_stop_reason=");
    }

    if let Some(lease) = &task.lease {
        println!("task_lease_id={}", lease.lease_id);
        println!("task_lease_worker_id={}", lease.worker_id);
        println!("task_lease_status={}", lease.status);
        println!(
            "task_lease_deadline_ms={}",
            lease
                .lease_deadline_ms
                .map_or_else(String::new, |deadline| deadline.to_string())
        );
        println!(
            "task_lease_reason={}",
            lease.reason.as_deref().unwrap_or("")
        );
    } else {
        println!("task_lease_id=");
        println!("task_lease_worker_id=");
        println!("task_lease_status=");
        println!("task_lease_deadline_ms=");
        println!("task_lease_reason=");
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

fn print_task_inspect_report(report: &TaskInspectReport) {
    println!("session_id={}", report.session_id);
    println!("task_id={}", report.task_id);
    println!("task_status={}", report.status);
    println!("task_repo_path={}", report.repo_path);
    println!("task_input_summary={}", report.input_summary.text);
    println!(
        "task_input_original_bytes={}",
        report.input_summary.original_bytes
    );
    println!("task_input_truncated={}", report.input_summary.truncated);
    println!("task_worker_lane_kind={}", report.worker_lane_kind);
    println!("task_context_max_bytes={}", report.context_budget.max_bytes);
    println!("task_context_max_files={}", report.context_budget.max_files);
    println!(
        "task_context_max_skill_files={}",
        report.context_budget.max_skill_files
    );
    println!("task_focus_terms={}", report.focus_terms.join(","));
    println!("task_max_output_tokens={}", report.max_output_tokens);

    if let Some(queue) = &report.queue {
        println!("task_queue_status={}", queue.status);
        println!(
            "task_queue_worker_id={}",
            queue.worker_id.as_deref().unwrap_or_default()
        );
        println!(
            "task_queue_lease_id={}",
            queue
                .lease_id
                .map_or_else(String::new, |lease_id| lease_id.to_string())
        );
        println!(
            "task_queue_lease_deadline_ms={}",
            queue
                .lease_deadline_ms
                .map_or_else(String::new, |deadline| deadline.to_string())
        );
        println!(
            "task_queue_retry_count={}",
            queue
                .retry_count
                .map_or_else(String::new, |count| count.to_string())
        );
        println!(
            "task_queue_max_retries={}",
            queue
                .max_retries
                .map_or_else(String::new, |retries| retries.to_string())
        );
        println!(
            "task_queue_stop_reason={}",
            queue.stop_reason.as_deref().unwrap_or_default()
        );
        println!(
            "task_queue_last_reason={}",
            queue.last_reason.as_deref().unwrap_or_default()
        );
    } else {
        println!("task_queue_status=");
        println!("task_queue_worker_id=");
        println!("task_queue_lease_id=");
        println!("task_queue_lease_deadline_ms=");
        println!("task_queue_retry_count=");
        println!("task_queue_max_retries=");
        println!("task_queue_stop_reason=");
        println!("task_queue_last_reason=");
    }

    if let Some(lease) = &report.lease {
        println!("task_lease_id={}", lease.lease_id);
        println!("task_lease_worker_id={}", lease.worker_id);
        println!("task_lease_status={}", lease.status);
        println!(
            "task_lease_deadline_ms={}",
            lease
                .lease_deadline_ms
                .map_or_else(String::new, |deadline| deadline.to_string())
        );
        println!(
            "task_lease_reason={}",
            lease.reason.as_deref().unwrap_or_default()
        );
    } else {
        println!("task_lease_id=");
        println!("task_lease_worker_id=");
        println!("task_lease_status=");
        println!("task_lease_deadline_ms=");
        println!("task_lease_reason=");
    }

    if let Some(approval) = &report.approval {
        println!("task_approval_state={}", approval.state);
        println!("task_approval_summary={}", approval.summary);
        println!(
            "task_approval_actor={}",
            approval.actor.as_deref().unwrap_or_default()
        );
        println!(
            "task_approval_reason={}",
            approval.reason.as_deref().unwrap_or_default()
        );
        println!(
            "task_approval_rejection_reason={}",
            approval.rejection_reason.as_deref().unwrap_or_default()
        );
    } else {
        println!("task_approval_state=");
        println!("task_approval_summary=");
        println!("task_approval_actor=");
        println!("task_approval_reason=");
        println!("task_approval_rejection_reason=");
    }

    if let Some(commit) = &report.commit {
        println!("task_commit_state={}", commit.state);
        println!(
            "task_commit_repo_path={}",
            commit.repo_path.as_deref().unwrap_or_default()
        );
        println!("task_commit_message={}", commit.message);
        println!(
            "task_commit_actor={}",
            commit.actor.as_deref().unwrap_or_default()
        );
        println!(
            "task_commit_sha={}",
            commit.commit_sha.as_deref().unwrap_or_default()
        );
        println!(
            "task_commit_failure_reason={}",
            commit.failure_reason.as_deref().unwrap_or_default()
        );
    } else {
        println!("task_commit_state=");
        println!("task_commit_repo_path=");
        println!("task_commit_message=");
        println!("task_commit_actor=");
        println!("task_commit_sha=");
        println!("task_commit_failure_reason=");
    }

    println!(
        "task_worker_workspace_path={}",
        report
            .workspace
            .worker_workspace_path
            .as_deref()
            .unwrap_or_default()
    );
    println!(
        "task_worker_worktree_path={}",
        report
            .workspace
            .worker_worktree_path
            .as_deref()
            .unwrap_or_default()
    );
    println!(
        "task_allocated_worktree_path={}",
        report
            .workspace
            .allocated_worktree_path
            .as_deref()
            .unwrap_or_default()
    );
    println!(
        "task_session_repo_path={}",
        report
            .workspace
            .session_repo_path
            .as_deref()
            .unwrap_or_default()
    );
    println!(
        "task_workspace_diff_repo_path={}",
        report
            .workspace
            .diff_repo_path
            .as_deref()
            .unwrap_or_default()
    );

    if let Some(diff) = &report.diff {
        println!(
            "task_diff_repo_path={}",
            diff.repo_path.as_deref().unwrap_or_default()
        );
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

    println!("task_event_count={}", report.event_summary.event_count);
    println!(
        "task_event_first_sequence={}",
        report
            .event_summary
            .first_sequence
            .map_or_else(String::new, |sequence| sequence.to_string())
    );
    println!(
        "task_event_latest_sequence={}",
        report
            .event_summary
            .latest_sequence
            .map_or_else(String::new, |sequence| sequence.to_string())
    );
    println!(
        "task_event_latest_type={}",
        report
            .event_summary
            .latest_event_type
            .as_deref()
            .unwrap_or_default()
    );
    println!("source_of_truth={}", report.source_of_truth);
    println!("projection_kind={}", report.projection_kind);
}

fn print_task_inspect_json(report: &TaskInspectReport) -> HarnessResult<()> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| HarnessError::new(error.to_string()))?;
    println!("{json}");
    Ok(())
}

fn print_task_queue_inspect_report(report: &TaskQueueInspectReport) {
    println!(
        "session_id={}",
        report
            .session_id
            .map_or_else(String::new, |session_id| session_id.to_string())
    );
    println!("queue_now_ms={}", report.now_ms);
    println!("queue_empty={}", report.empty);
    println!("queue_total_count={}", report.total_count);
    println!("queue_active_leased_count={}", report.active_leased_count);
    println!(
        "queue_expired_looking_leased_count={}",
        report.expired_looking_leased_count
    );
    println!(
        "queue_status_counts={}",
        format_task_queue_counts(&report.queue_status_counts)
    );
    println!(
        "queue_status_class_counts={}",
        format_task_queue_counts(&report.status_class_counts)
    );
    println!(
        "queue_task_status_counts={}",
        format_task_queue_counts(&report.task_status_counts)
    );
    println!(
        "queue_lease_state_counts={}",
        format_task_queue_counts(&report.lease_state_counts)
    );
    println!("source_of_truth={}", report.source_of_truth);
    println!("projection_kind={}", report.projection_kind);

    for (index, task) in report.tasks.iter().enumerate() {
        println!("queue_task_{index}_session_id={}", task.session_id);
        println!("queue_task_{index}_id={}", task.task_id);
        println!(
            "queue_task_{index}_task_status={}",
            task.task_status.as_deref().unwrap_or_default()
        );
        println!("queue_task_{index}_queue_status={}", task.queue_status);
        println!("queue_task_{index}_status_class={}", task.status_class);
        println!("queue_task_{index}_lease_state={}", task.lease_state);
        println!(
            "queue_task_{index}_worker_id={}",
            task.worker_id.as_deref().unwrap_or_default()
        );
        println!(
            "queue_task_{index}_lease_id={}",
            task.lease_id
                .map_or_else(String::new, |lease_id| lease_id.to_string())
        );
        println!(
            "queue_task_{index}_lease_deadline_ms={}",
            task.lease_deadline_ms
                .map_or_else(String::new, |deadline| deadline.to_string())
        );
        println!("queue_task_{index}_retry_count={}", task.retry_count);
        println!("queue_task_{index}_max_retries={}", task.max_retries);
        println!(
            "queue_task_{index}_stop_reason={}",
            task.stop_reason.as_deref().unwrap_or_default()
        );
        println!(
            "queue_task_{index}_last_reason={}",
            task.last_reason.as_deref().unwrap_or_default()
        );
    }
}

fn print_task_queue_inspect_json(report: &TaskQueueInspectReport) -> HarnessResult<()> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| HarnessError::new(error.to_string()))?;
    println!("{json}");
    Ok(())
}

fn format_task_queue_counts(counts: &[TaskQueueInspectStatusCount]) -> String {
    counts
        .iter()
        .map(|count| format!("{}:{}", count.status, count.count))
        .collect::<Vec<_>>()
        .join(",")
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

fn print_model_provider_routing_result(result: &ModelProviderRoutingResult) {
    println!("session_id={}", result.session_id);
    println!("model_provider={}", result.provider);
    println!("model_provider_kind={}", result.provider_kind);
    println!("model_id={}", result.model_id);
    println!("model_status={}", result.status.as_str());
    println!("usage_known={}", result.usage_known);
    println!("max_output_tokens={}", result.max_output_tokens);

    if let Some(summary) = &result.summary {
        println!("model_summary={summary}");
    }

    if let Some(skipped_reason) = &result.skipped_reason {
        println!("model_skipped_reason={skipped_reason}");
    }

    if let Some(prompt_tokens) = result.prompt_tokens {
        println!("prompt_tokens={prompt_tokens}");
    }

    if let Some(completion_tokens) = result.completion_tokens {
        println!("completion_tokens={completion_tokens}");
    }

    if let Some(patch) = &result.patch {
        println!("patch_path={}", patch.path);
    }

    println!("event_count={}", result.event_count);
}

fn print_fake_model_turn_result(result: &FakeModelTurnResult) {
    println!("session_id={}", result.session_id);
    println!("model_provider={}", result.provider);
    println!("model_provider_kind={}", result.provider_kind);
    println!("model_id={}", result.model_id);
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

fn print_leased_codex_worker_task_result(result: &LeasedCodexWorkerTaskResult) {
    println!("lease_id={}", result.lease_id);
    println!("worker_lane_id={}", result.worker.lane_id);
    println!(
        "worker_final_status={}",
        result.worker.final_status.as_str()
    );
    println!(
        "worker_pending_commit_state={}",
        result.worker.pending_commit_state.as_deref().unwrap_or("")
    );

    if let Some(observation) = &result.worker.observation {
        if let Some(exit_code) = observation.exit_code {
            println!("worker_exit_code={exit_code}");
        } else {
            println!("worker_exit_code=signal");
        }
        println!("worker_duration_ms={}", observation.duration_ms);
    }

    println!(
        "event_replay_total={}",
        result.worker.event_replay.total_events
    );
    println!(
        "event_replay_last={}",
        result
            .worker
            .event_replay
            .last_event_type
            .as_deref()
            .unwrap_or("")
    );
    print_task_projection(&result.task);
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
