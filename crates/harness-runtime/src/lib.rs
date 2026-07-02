use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

pub use harness_context::ContextBudget;
use harness_context::{CompiledContextBundle, ContextCompileRequest, compile_repository_context};
use harness_core::{HarnessError, HarnessResult};
use harness_db::PostgresEventStore;
use harness_events::{
    CommitApprovalDecisionPayload, CommitApprovalPendingPayload, CommitHandoffPayload,
    ContextCompiledPayload, ContextSourcePayload, DiffSummaryPayload, EventEnvelope, EventType,
    FilePatchIntentPayload, FilePatchObservationPayload, FilePatchPayload, ModelDecisionPayload,
    ModelRequestPayload, NewEvent, PolicyDecisionPayload, RecoveryFailurePayload,
    RecoveryPlanPayload, RecoveryRepairAttemptPayload, RecoveryStoppedPayload,
    SessionStartedPayload, SkillMetadataPayload, TaskCreatedPayload, ToolCallIntentPayload,
    ToolObservationPayload, WorkerLaneBudgetPayload, WorkerLaneObservationPayload,
    WorkerLaneRequestPayload, WorkerLaneStatePayload, WorkerLaneWorktreeAllocatedPayload,
};
use harness_models::{DeterministicFakeModelProvider, FakeModelRequest, FilePatchProposal};
use harness_policy::{
    CodexWorkerLanePolicyInput, PolicyDecision, evaluate_codex_worker_lane, evaluate_file_patch,
    evaluate_verification_command,
};
pub use harness_tools::{
    CodexWorkerLaneBudget, CodexWorkerLaneFixture, CodexWorkerLaneObservation,
    CodexWorkerLaneRunner, CodexWorkerSubprocess, CodexWorkerUsage, UsageConfidence,
    WorkerLaneStatus,
};
use harness_tools::{
    CodexWorkerLaneFixtureAdapter, CodexWorkerLaneIntent, CommandObservation, CommandToolRuntime,
    FilePatchIntent, FilePatchObservation, OneShotWorkerRunner, VerifyCommandIntent,
};
use uuid::Uuid;

pub const PENDING_COMMIT_APPROVAL_STATE: &str = "pending_commit_approval";
pub const COMMIT_APPROVED_STATE: &str = "approved";
pub const COMMIT_REJECTED_STATE: &str = "rejected";
pub const COMMITTING_STATE: &str = "committing";
pub const COMMITTED_STATE: &str = "committed";
pub const COMMIT_FAILED_STATE: &str = "commit_failed";
pub const TASK_CREATED_STATE: &str = "created";
pub const DEFAULT_TASK_WORKER_LANE_KIND: &str = "codex_cli_worker_lane";
pub const RECOVERY_FAILED_STATE: &str = "recovery_failed";
pub const SELF_RECOVERY_MAX_ROUNDS: usize = 2;
pub const DEFAULT_CODEX_CLI_PROGRAM: &str = "codex";
pub const DEFAULT_CODEX_CLI_ACCEPTANCE_TIMEOUT_MS: u64 = 120_000;
pub const DEFAULT_CODEX_CLI_ACCEPTANCE_TASK: &str = "Create or update .harness/codex-manual-acceptance.md with a short note saying local Codex CLI manual acceptance ran successfully. Do not commit the change.";

pub struct Runtime {
    event_store: PostgresEventStore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartSessionRequest {
    pub repo_path: String,
}

impl StartSessionRequest {
    #[must_use]
    pub fn new(repo_path: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Started,
}

impl SessionStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionProjection {
    pub session_id: Uuid,
    pub repo_path: String,
    pub status: SessionStatus,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTaskRequest {
    pub input: String,
    pub repo_path: Option<String>,
    pub worker_lane_kind: String,
    pub context: SessionContextCompileRequest,
    pub max_output_tokens: usize,
}

impl CreateTaskRequest {
    #[must_use]
    pub fn new(input: impl Into<String>) -> Self {
        Self {
            input: input.into(),
            repo_path: None,
            worker_lane_kind: DEFAULT_TASK_WORKER_LANE_KIND.to_owned(),
            context: SessionContextCompileRequest::default(),
            max_output_tokens: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationCommandRequest {
    pub program: String,
    pub args: Vec<String>,
}

impl VerificationCommandRequest {
    #[must_use]
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationCommandResult {
    pub session_id: Uuid,
    pub decision: PolicyDecision,
    pub reason: String,
    pub observation: Option<CommandObservation>,
    pub event_count: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SessionContextCompileRequest {
    pub budget: ContextBudget,
    pub focus_terms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContextCompileResult {
    pub session_id: Uuid,
    pub bundle: CompiledContextBundle,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeModelTurnRequest {
    pub task: String,
    pub context: SessionContextCompileRequest,
    pub max_output_tokens: usize,
}

impl FakeModelTurnRequest {
    #[must_use]
    pub fn new(task: impl Into<String>) -> Self {
        Self {
            task: task.into(),
            context: SessionContextCompileRequest::default(),
            max_output_tokens: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeModelTurnResult {
    pub session_id: Uuid,
    pub patch: FilePatchProposal,
    pub decision: PolicyDecision,
    pub reason: String,
    pub observation: Option<FilePatchObservation>,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmallCodingTaskRequest {
    pub task: String,
    pub context: SessionContextCompileRequest,
    pub max_output_tokens: usize,
    pub verification: VerificationCommandRequest,
}

impl SmallCodingTaskRequest {
    #[must_use]
    pub fn new(task: impl Into<String>, verification: VerificationCommandRequest) -> Self {
        Self {
            task: task.into(),
            context: SessionContextCompileRequest::default(),
            max_output_tokens: 256,
            verification,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSummary {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalState {
    Pending,
    Approved,
    Rejected,
}

impl ApprovalState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => PENDING_COMMIT_APPROVAL_STATE,
            Self::Approved => COMMIT_APPROVED_STATE,
            Self::Rejected => COMMIT_REJECTED_STATE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalProjection {
    pub session_id: Uuid,
    pub state: String,
    pub diff: DiffSummary,
    pub summary: String,
    pub rejection_reason: Option<String>,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitHandoffProjection {
    pub session_id: Uuid,
    pub state: String,
    pub repo_path: String,
    pub message: String,
    pub commit_sha: Option<String>,
    pub failure_reason: Option<String>,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskWorktreeProjection {
    pub lane_id: String,
    pub lane_kind: String,
    pub worktree_path: String,
    pub base_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskWorkerOutputProjection {
    pub lane_id: String,
    pub lane_kind: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskApprovalProjection {
    pub state: String,
    pub summary: String,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCommitProjection {
    pub state: String,
    pub message: String,
    pub commit_sha: Option<String>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskProjection {
    pub session_id: Uuid,
    pub task_id: Uuid,
    pub repo_path: String,
    pub input: String,
    pub status: String,
    pub worker_lane_kind: String,
    pub context_budget: ContextBudget,
    pub focus_terms: Vec<String>,
    pub max_output_tokens: usize,
    pub worktree: Option<TaskWorktreeProjection>,
    pub worker_output: Option<TaskWorkerOutputProjection>,
    pub diff: Option<DiffSummary>,
    pub approval: Option<TaskApprovalProjection>,
    pub commit: Option<TaskCommitProjection>,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitDiffEvidence {
    summary: DiffSummary,
    git_status: String,
    git_diff: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventReplaySummary {
    pub total_events: usize,
    pub last_event_type: Option<String>,
    pub event_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenLedgerSummary {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
    pub max_output_tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmallCodingTaskResult {
    pub session_id: Uuid,
    pub patch: FilePatchProposal,
    pub patch_applied: bool,
    pub verification: VerificationCommandResult,
    pub diff: DiffSummary,
    pub event_replay: EventReplaySummary,
    pub token_ledger: TokenLedgerSummary,
    pub final_state: String,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfRecoveryLoopRequest {
    pub task: String,
    pub context: SessionContextCompileRequest,
    pub max_output_tokens: usize,
    pub verification: VerificationCommandRequest,
    pub max_recovery_rounds: usize,
    pub max_repair_bytes: usize,
}

impl SelfRecoveryLoopRequest {
    #[must_use]
    pub fn new(task: impl Into<String>, verification: VerificationCommandRequest) -> Self {
        Self {
            task: task.into(),
            context: SessionContextCompileRequest::default(),
            max_output_tokens: 256,
            verification,
            max_recovery_rounds: SELF_RECOVERY_MAX_ROUNDS,
            max_repair_bytes: 16 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryStopReason {
    Recovered,
    MaxRoundsReached,
    RepairBudgetExceeded,
}

impl RecoveryStopReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recovered => "recovered",
            Self::MaxRoundsReached => "max_rounds_reached",
            Self::RepairBudgetExceeded => "repair_budget_exceeded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfRecoveryReport {
    pub failure_classification: Option<String>,
    pub recovery_plan: Option<String>,
    pub repair_attempts: usize,
    pub retry_count: usize,
    pub stop_reason: RecoveryStopReason,
    pub max_recovery_rounds: usize,
    pub max_repair_bytes: usize,
    pub used_repair_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfRecoveryLoopResult {
    pub session_id: Uuid,
    pub initial_verification: VerificationCommandResult,
    pub final_verification: VerificationCommandResult,
    pub report: SelfRecoveryReport,
    pub diff: DiffSummary,
    pub event_replay: EventReplaySummary,
    pub token_ledger: TokenLedgerSummary,
    pub final_state: String,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexWorkerLaneRequest {
    pub task: String,
    pub workspace: CodexWorkerLaneWorkspace,
    pub timeout_ms: u64,
    pub cancellation_requested: bool,
    pub budget: CodexWorkerLaneBudget,
    pub runner: CodexWorkerLaneRunner,
}

impl CodexWorkerLaneRequest {
    #[must_use]
    pub fn new(task: impl Into<String>, fixture: CodexWorkerLaneFixture) -> Self {
        Self::new_runner(task, CodexWorkerLaneRunner::fixture(fixture))
    }

    #[must_use]
    pub fn new_subprocess(task: impl Into<String>, subprocess: CodexWorkerSubprocess) -> Self {
        Self::new_runner(task, CodexWorkerLaneRunner::subprocess(subprocess))
    }

    #[must_use]
    pub fn new_runner(task: impl Into<String>, runner: CodexWorkerLaneRunner) -> Self {
        Self {
            task: task.into(),
            workspace: CodexWorkerLaneWorkspace::default(),
            timeout_ms: 30_000,
            cancellation_requested: false,
            budget: CodexWorkerLaneBudget::default(),
            runner,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CodexWorkerLaneWorkspace {
    #[default]
    AllocateTaskWorktree,
    DangerousCurrentWorkspace,
    ExistingWorktree {
        path: String,
    },
}

impl CodexWorkerLaneWorkspace {
    #[must_use]
    pub const fn allocate_task_worktree() -> Self {
        Self::AllocateTaskWorktree
    }

    #[must_use]
    pub const fn dangerous_current_workspace() -> Self {
        Self::DangerousCurrentWorkspace
    }

    #[must_use]
    pub fn existing_worktree(path: impl Into<String>) -> Self {
        Self::ExistingWorktree { path: path.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexWorkerLaneResult {
    pub session_id: Uuid,
    pub lane_id: String,
    pub decision: PolicyDecision,
    pub reason: String,
    pub observation: Option<CodexWorkerLaneObservation>,
    pub final_status: WorkerLaneStatus,
    pub pending_commit_state: Option<String>,
    pub event_replay: EventReplaySummary,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexCliAvailabilityRequest {
    pub program: String,
    pub codex_home: Option<PathBuf>,
    pub codex_api_key_present: bool,
}

impl CodexCliAvailabilityRequest {
    #[must_use]
    pub fn from_env(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            codex_home: env::var("CODEX_HOME")
                .ok()
                .map(PathBuf::from)
                .or_else(default_codex_home),
            codex_api_key_present: env::var("CODEX_API_KEY")
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexCliAvailability {
    pub program: String,
    pub available: bool,
    pub authenticated: bool,
    pub version: Option<String>,
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexCliAcceptanceStatus {
    Skipped,
    Ran,
}

impl CodexCliAcceptanceStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Skipped => "skipped",
            Self::Ran => "ran",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexCliAcceptanceRequest {
    pub task: String,
    pub codex_program: String,
    pub timeout_ms: u64,
    pub max_stdout_bytes: usize,
    pub availability: CodexCliAvailabilityRequest,
}

impl CodexCliAcceptanceRequest {
    #[must_use]
    pub fn from_env(task: impl Into<String>, codex_program: impl Into<String>) -> Self {
        let codex_program = codex_program.into();

        Self {
            task: task.into(),
            codex_program: codex_program.clone(),
            timeout_ms: DEFAULT_CODEX_CLI_ACCEPTANCE_TIMEOUT_MS,
            max_stdout_bytes: 128 * 1024,
            availability: CodexCliAvailabilityRequest::from_env(codex_program),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexCliAcceptanceResult {
    pub session_id: Uuid,
    pub status: CodexCliAcceptanceStatus,
    pub availability: CodexCliAvailability,
    pub worker: Option<CodexWorkerLaneResult>,
}

impl Runtime {
    #[must_use]
    pub fn new(event_store: PostgresEventStore) -> Self {
        Self { event_store }
    }

    pub fn connect_postgres(database_url: &str) -> HarnessResult<Self> {
        Ok(Self::new(PostgresEventStore::connect(database_url)?))
    }

    pub fn start_session(
        &mut self,
        request: StartSessionRequest,
    ) -> HarnessResult<SessionProjection> {
        if request.repo_path.trim().is_empty() {
            return Err(HarnessError::new("repo path cannot be empty"));
        }

        let session_id = Uuid::new_v4();
        let event =
            NewEvent::session_started(session_id, SessionStartedPayload::new(request.repo_path))?;

        self.event_store.append_event(event)?;
        self.show_session(session_id)
    }

    pub fn show_session(&self, session_id: Uuid) -> HarnessResult<SessionProjection> {
        let events = self.event_store.events_for_session(session_id)?;
        project_session(session_id, &events)
    }

    pub fn create_task(
        &mut self,
        session_id: Uuid,
        request: CreateTaskRequest,
    ) -> HarnessResult<TaskProjection> {
        if request.input.trim().is_empty() {
            return Err(HarnessError::new("task input cannot be empty"));
        }

        if request.worker_lane_kind.trim().is_empty() {
            return Err(HarnessError::new("task worker lane kind cannot be empty"));
        }

        let session = self.show_session(session_id)?;
        let repo_path = request
            .repo_path
            .unwrap_or_else(|| session.repo_path.clone());
        if repo_path.trim().is_empty() {
            return Err(HarnessError::new("task repo path cannot be empty"));
        }

        let task_id = Uuid::new_v4();
        self.event_store.append_event(NewEvent::task_created(
            session_id,
            TaskCreatedPayload::new(
                task_id,
                repo_path,
                request.input.trim(),
                TASK_CREATED_STATE,
                request.worker_lane_kind.trim(),
                request.context.budget.max_bytes,
                request.context.budget.max_files,
                request.context.budget.max_skill_files,
                request.context.focus_terms,
                request.max_output_tokens,
            ),
        )?)?;

        self.show_task(session_id, task_id)
    }

    pub fn list_tasks(&self, session_id: Uuid) -> HarnessResult<Vec<TaskProjection>> {
        let events = self.event_store.events_for_session(session_id)?;
        project_session(session_id, &events)?;
        task_projections_from_events(session_id, &events)
    }

    pub fn show_task(&self, session_id: Uuid, task_id: Uuid) -> HarnessResult<TaskProjection> {
        let tasks = self.list_tasks(session_id)?;
        tasks
            .into_iter()
            .find(|task| task.task_id == task_id)
            .ok_or_else(|| HarnessError::new(format!("task not found: {task_id}")))
    }

    pub fn events_for_session(&self, session_id: Uuid) -> HarnessResult<Vec<EventEnvelope>> {
        self.event_store.events_for_session(session_id)
    }

    pub fn show_approval(&self, session_id: Uuid) -> HarnessResult<ApprovalProjection> {
        let events = self.event_store.events_for_session(session_id)?;

        approval_projection_from_events(session_id, &events)
    }

    pub fn approve_pending_diff(&mut self, session_id: Uuid) -> HarnessResult<ApprovalProjection> {
        let projection = self.show_approval(session_id)?;
        if projection.state != PENDING_COMMIT_APPROVAL_STATE {
            return Err(HarnessError::new(format!(
                "pending diff is already {}",
                projection.state
            )));
        }

        self.event_store.append_event(NewEvent::commit_approved(
            session_id,
            CommitApprovalDecisionPayload::new(
                COMMIT_APPROVED_STATE,
                "approved by runtime approval state machine",
                "runtime",
            ),
        )?)?;

        self.show_approval(session_id)
    }

    pub fn reject_pending_diff(
        &mut self,
        session_id: Uuid,
        reason: impl Into<String>,
    ) -> HarnessResult<ApprovalProjection> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(HarnessError::new("rejection reason is required"));
        }

        let projection = self.show_approval(session_id)?;
        if projection.state != PENDING_COMMIT_APPROVAL_STATE {
            return Err(HarnessError::new(format!(
                "pending diff is already {}",
                projection.state
            )));
        }

        self.event_store.append_event(NewEvent::commit_rejected(
            session_id,
            CommitApprovalDecisionPayload::new(COMMIT_REJECTED_STATE, reason.trim(), "runtime"),
        )?)?;

        self.show_approval(session_id)
    }

    pub fn commit_approved_diff(
        &mut self,
        session_id: Uuid,
        message: impl Into<String>,
    ) -> HarnessResult<CommitHandoffProjection> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(HarnessError::new("commit message is required"));
        }

        let approval = self.show_approval(session_id)?;
        if approval.state != COMMIT_APPROVED_STATE {
            return Err(HarnessError::new(format!(
                "approved diff is required before commit; current state is {}",
                approval.state
            )));
        }

        let events = self.event_store.events_for_session(session_id)?;
        if let Some(state) = existing_commit_handoff_state_from_events(&events)? {
            return Err(HarnessError::new(format!(
                "commit handoff is already {state}"
            )));
        }

        let repo_path = commit_repo_path_from_events(&events)?;
        self.event_store.append_event(NewEvent::commit_started(
            session_id,
            CommitHandoffPayload::started(&repo_path, message.trim(), "runtime"),
        )?)?;

        match run_harness_commit(&repo_path, message.trim()) {
            Ok(commit_sha) => {
                self.event_store.append_event(NewEvent::commit_succeeded(
                    session_id,
                    CommitHandoffPayload::succeeded(
                        &repo_path,
                        message.trim(),
                        "runtime",
                        commit_sha,
                    ),
                )?)?;
            }
            Err(error) => {
                self.event_store.append_event(NewEvent::commit_failed(
                    session_id,
                    CommitHandoffPayload::failed(
                        &repo_path,
                        message.trim(),
                        "runtime",
                        error.to_string(),
                    ),
                )?)?;
            }
        }

        let events = self.event_store.events_for_session(session_id)?;
        commit_handoff_projection_from_events(session_id, &events)
    }

    pub fn run_verification_command(
        &mut self,
        session_id: Uuid,
        request: VerificationCommandRequest,
    ) -> HarnessResult<VerificationCommandResult> {
        let session = self.show_session(session_id)?;
        let intent =
            VerifyCommandIntent::new(request.program, request.args, session.repo_path.clone())?;
        let tool_name = harness_tools::VERIFY_COMMAND_TOOL.name;

        self.event_store.append_event(NewEvent::tool_call_intended(
            session_id,
            ToolCallIntentPayload::new(
                tool_name,
                intent.program.clone(),
                intent.args.clone(),
                intent.working_dir.clone(),
            ),
        )?)?;

        let evaluation = evaluate_verification_command(&intent.program, &intent.args);

        self.event_store.append_event(NewEvent::policy_decided(
            session_id,
            PolicyDecisionPayload::new(
                tool_name,
                evaluation.decision.as_str(),
                evaluation.reason.clone(),
            ),
        )?)?;

        let observation = if evaluation.decision == PolicyDecision::Allow {
            let observation = CommandToolRuntime.run_verify_command(&intent)?;

            self.event_store
                .append_event(NewEvent::tool_observation_recorded(
                    session_id,
                    ToolObservationPayload::new(
                        tool_name,
                        observation.exit_code,
                        observation.stdout.clone(),
                        observation.stderr.clone(),
                        observation.duration_ms,
                    ),
                )?)?;

            Some(observation)
        } else {
            None
        };

        Ok(VerificationCommandResult {
            session_id,
            decision: evaluation.decision,
            reason: evaluation.reason,
            observation,
            event_count: self.event_store.events_for_session(session_id)?.len(),
        })
    }

    pub fn compile_session_context(
        &mut self,
        session_id: Uuid,
        request: SessionContextCompileRequest,
    ) -> HarnessResult<SessionContextCompileResult> {
        let session = self.show_session(session_id)?;
        let bundle = compile_repository_context(ContextCompileRequest {
            repo_path: session.repo_path.into(),
            budget: request.budget,
            focus_terms: request.focus_terms,
        })?;

        self.event_store.append_event(NewEvent::context_compiled(
            session_id,
            context_compiled_payload(&bundle),
        )?)?;

        Ok(SessionContextCompileResult {
            session_id,
            bundle,
            event_count: self.event_store.events_for_session(session_id)?.len(),
        })
    }

    pub fn run_fake_model_turn(
        &mut self,
        session_id: Uuid,
        request: FakeModelTurnRequest,
    ) -> HarnessResult<FakeModelTurnResult> {
        let context = self.compile_session_context(session_id, request.context)?;
        let model_request = FakeModelRequest::new(
            request.task,
            context.bundle.sources.len(),
            context.bundle.used_bytes,
            request.max_output_tokens,
        )?;
        let provider = DeterministicFakeModelProvider;

        self.event_store
            .append_event(NewEvent::model_request_recorded(
                session_id,
                ModelRequestPayload::new(
                    "deterministic-fake-model",
                    model_request.task.clone(),
                    model_request.context_source_count,
                    model_request.context_used_bytes,
                    model_request.max_output_tokens,
                ),
            )?)?;

        let decision = provider.decide(model_request)?;
        let patch_payload = file_patch_payload(&decision.patch);

        self.event_store
            .append_event(NewEvent::model_decision_recorded(
                session_id,
                ModelDecisionPayload::new(
                    decision.provider,
                    decision.summary.clone(),
                    decision.usage.prompt_tokens,
                    decision.usage.completion_tokens,
                    decision.usage.max_output_tokens,
                    patch_payload.clone(),
                ),
            )?)?;

        let tool_name = harness_tools::APPLY_FILE_PATCH_TOOL.name;
        let intent = FilePatchIntent::new(
            context.bundle.repo_path.clone(),
            decision.patch.path.clone(),
            decision.patch.expected_content.clone(),
            decision.patch.replacement_content.clone(),
        )?;

        self.event_store
            .append_event(NewEvent::file_patch_intended(
                session_id,
                FilePatchIntentPayload::new(
                    tool_name,
                    context.bundle.repo_path.clone(),
                    patch_payload,
                ),
            )?)?;

        let evaluation = evaluate_file_patch(
            &decision.patch.path,
            decision.patch.replacement_content.len(),
        );

        self.event_store.append_event(NewEvent::policy_decided(
            session_id,
            PolicyDecisionPayload::new(
                tool_name,
                evaluation.decision.as_str(),
                evaluation.reason.clone(),
            ),
        )?)?;

        let observation = if evaluation.decision == PolicyDecision::Allow {
            let observation = CommandToolRuntime.run_file_patch(&intent)?;

            self.event_store
                .append_event(NewEvent::file_patch_observation_recorded(
                    session_id,
                    FilePatchObservationPayload::new(
                        tool_name,
                        observation.path.clone(),
                        observation.applied,
                        observation.previous_bytes,
                        observation.new_bytes,
                        observation.duration_ms,
                    ),
                )?)?;

            Some(observation)
        } else {
            None
        };

        Ok(FakeModelTurnResult {
            session_id,
            patch: decision.patch,
            decision: evaluation.decision,
            reason: evaluation.reason,
            observation,
            prompt_tokens: decision.usage.prompt_tokens,
            completion_tokens: decision.usage.completion_tokens,
            event_count: self.event_store.events_for_session(session_id)?.len(),
        })
    }

    pub fn run_small_coding_task(
        &mut self,
        session_id: Uuid,
        request: SmallCodingTaskRequest,
    ) -> HarnessResult<SmallCodingTaskResult> {
        let fake_turn = self.run_fake_model_turn(
            session_id,
            FakeModelTurnRequest {
                task: request.task,
                context: request.context,
                max_output_tokens: request.max_output_tokens,
            },
        )?;

        if fake_turn.observation.is_none() {
            return Err(HarnessError::new("coding task patch was not applied"));
        }

        let verification = self.run_verification_command(session_id, request.verification)?;
        let Some(observation) = &verification.observation else {
            return Err(HarnessError::new("verification command was not executed"));
        };

        if observation.exit_code != Some(0) {
            return Err(HarnessError::new("verification command failed"));
        }

        let diff = diff_summary_from_patch(&fake_turn.patch);
        self.event_store.append_event(NewEvent::diff_recorded(
            session_id,
            diff_summary_payload(&diff),
        )?)?;
        self.event_store
            .append_event(NewEvent::commit_approval_pending(
                session_id,
                CommitApprovalPendingPayload::new(
                    PENDING_COMMIT_APPROVAL_STATE,
                    "verification passed; awaiting human commit approval",
                ),
            )?)?;

        let events = self.event_store.events_for_session(session_id)?;
        let event_replay = event_replay_summary(&events);

        Ok(SmallCodingTaskResult {
            session_id,
            patch: fake_turn.patch,
            patch_applied: true,
            verification,
            diff,
            event_replay,
            token_ledger: TokenLedgerSummary {
                prompt_tokens: fake_turn.prompt_tokens,
                completion_tokens: fake_turn.completion_tokens,
                total_tokens: fake_turn.prompt_tokens + fake_turn.completion_tokens,
                max_output_tokens: request.max_output_tokens,
            },
            final_state: PENDING_COMMIT_APPROVAL_STATE.to_owned(),
            event_count: events.len(),
        })
    }

    pub fn run_self_recovery_fixture_task(
        &mut self,
        session_id: Uuid,
        request: SelfRecoveryLoopRequest,
    ) -> HarnessResult<SelfRecoveryLoopResult> {
        let max_recovery_rounds = request.max_recovery_rounds.min(SELF_RECOVERY_MAX_ROUNDS);
        let session = self.show_session(session_id)?;
        let fake_turn = self.run_fake_model_turn(
            session_id,
            FakeModelTurnRequest {
                task: request.task,
                context: request.context,
                max_output_tokens: request.max_output_tokens,
            },
        )?;

        if fake_turn.observation.is_none() {
            return Err(HarnessError::new("recovery fixture patch was not applied"));
        }

        let initial_verification =
            self.run_verification_command(session_id, request.verification.clone())?;
        let Some(initial_observation) = &initial_verification.observation else {
            return Err(HarnessError::new(
                "initial verification command was not executed",
            ));
        };

        if initial_observation.exit_code == Some(0) {
            let diff = diff_summary_from_patch(&fake_turn.patch);
            return self.finish_recovery_loop(RecoveryFinish {
                session_id,
                initial_verification: initial_verification.clone(),
                final_verification: initial_verification,
                fake_turn,
                diff,
                failure_classification: None,
                recovery_plan: None,
                repair_attempts: 0,
                retry_count: 0,
                stop_reason: RecoveryStopReason::Recovered,
                max_recovery_rounds,
                max_repair_bytes: request.max_repair_bytes,
                used_repair_bytes: 0,
                max_output_tokens: request.max_output_tokens,
                final_state: PENDING_COMMIT_APPROVAL_STATE,
            });
        }

        let classification = classify_recovery_failure(initial_observation);
        self.event_store
            .append_event(NewEvent::recovery_failure_classified(
                session_id,
                RecoveryFailurePayload::new(
                    0,
                    classification.clone(),
                    initial_observation.exit_code,
                    "fixture verification failed before recovery",
                ),
            )?)?;

        let mut final_verification = initial_verification.clone();
        let mut last_repair_patch = fake_turn.patch.clone();
        let mut recovery_plan = None;
        let mut repair_attempts = 0;
        let mut retry_count = 0;
        let mut used_repair_bytes = 0;

        for round in 1..=max_recovery_rounds {
            let repair_patch = recovery_repair_patch(&session.repo_path, &fake_turn.patch)?;
            let repair_bytes = repair_patch.replacement_content.len();
            let remaining_repair_bytes = request.max_repair_bytes.saturating_sub(used_repair_bytes);
            let plan = "append recovered=true marker to the fake model patch".to_owned();
            recovery_plan = Some(plan.clone());
            self.event_store
                .append_event(NewEvent::recovery_plan_recorded(
                    session_id,
                    RecoveryPlanPayload::new(
                        round,
                        plan,
                        max_recovery_rounds,
                        remaining_repair_bytes,
                    ),
                )?)?;

            if repair_bytes > remaining_repair_bytes {
                return self.finish_recovery_loop(RecoveryFinish {
                    session_id,
                    initial_verification: initial_verification.clone(),
                    final_verification,
                    fake_turn,
                    diff: diff_summary_from_patch(&repair_patch),
                    failure_classification: Some(classification),
                    recovery_plan,
                    repair_attempts,
                    retry_count,
                    stop_reason: RecoveryStopReason::RepairBudgetExceeded,
                    max_recovery_rounds,
                    max_repair_bytes: request.max_repair_bytes,
                    used_repair_bytes,
                    max_output_tokens: request.max_output_tokens,
                    final_state: RECOVERY_FAILED_STATE,
                });
            }

            let repair_observation =
                self.apply_file_patch_with_events(session_id, &session.repo_path, &repair_patch)?;
            let repair_applied = repair_observation.is_some();
            repair_attempts += usize::from(repair_applied);
            used_repair_bytes += repair_bytes;
            self.event_store
                .append_event(NewEvent::recovery_repair_attempted(
                    session_id,
                    RecoveryRepairAttemptPayload::new(
                        round,
                        file_patch_payload(&repair_patch),
                        repair_applied,
                        repair_bytes,
                    ),
                )?)?;

            final_verification =
                self.run_verification_command(session_id, request.verification.clone())?;
            retry_count += 1;
            let final_success = final_verification
                .observation
                .as_ref()
                .and_then(|observation| observation.exit_code)
                == Some(0);
            last_repair_patch = repair_patch;

            if final_success {
                return self.finish_recovery_loop(RecoveryFinish {
                    session_id,
                    initial_verification,
                    final_verification,
                    fake_turn,
                    diff: diff_summary_from_patch(&last_repair_patch),
                    failure_classification: Some(classification),
                    recovery_plan,
                    repair_attempts,
                    retry_count,
                    stop_reason: RecoveryStopReason::Recovered,
                    max_recovery_rounds,
                    max_repair_bytes: request.max_repair_bytes,
                    used_repair_bytes,
                    max_output_tokens: request.max_output_tokens,
                    final_state: PENDING_COMMIT_APPROVAL_STATE,
                });
            }
        }

        self.finish_recovery_loop(RecoveryFinish {
            session_id,
            initial_verification,
            final_verification,
            fake_turn,
            diff: diff_summary_from_patch(&last_repair_patch),
            failure_classification: Some(classification),
            recovery_plan,
            repair_attempts,
            retry_count,
            stop_reason: RecoveryStopReason::MaxRoundsReached,
            max_recovery_rounds,
            max_repair_bytes: request.max_repair_bytes,
            used_repair_bytes,
            max_output_tokens: request.max_output_tokens,
            final_state: RECOVERY_FAILED_STATE,
        })
    }

    pub fn run_codex_worker_lane(
        &mut self,
        session_id: Uuid,
        request: CodexWorkerLaneRequest,
    ) -> HarnessResult<CodexWorkerLaneResult> {
        let session = self.show_session(session_id)?;

        let lane_id = Uuid::new_v4().to_string();
        let lane_kind = "codex_cli";
        let tool_name = harness_tools::CODEX_WORKER_LANE_TOOL.name;
        let workspace = requested_codex_worker_workspace(&request.workspace, &session.repo_path);
        let intent = CodexWorkerLaneIntent {
            task: request.task,
            workspace_path: workspace.workspace_path,
            worktree_path: workspace.worktree_path,
            timeout_ms: request.timeout_ms,
            cancellation_requested: request.cancellation_requested,
            budget: request.budget,
        };

        self.event_store
            .append_event(NewEvent::worker_lane_requested(
                session_id,
                worker_lane_request_payload(&lane_id, lane_kind, &intent),
            )?)?;

        let evaluation = evaluate_codex_worker_lane(&CodexWorkerLanePolicyInput {
            task: &intent.task,
            session_repo_path: &session.repo_path,
            workspace_path: &intent.workspace_path,
            worktree_path: intent.worktree_path.as_deref(),
            timeout_ms: intent.timeout_ms,
            max_prompt_tokens: intent.budget.max_prompt_tokens,
            max_output_tokens: intent.budget.max_output_tokens,
            max_stdout_bytes: intent.budget.max_stdout_bytes,
        });

        self.event_store.append_event(NewEvent::policy_decided(
            session_id,
            PolicyDecisionPayload::new(
                tool_name,
                evaluation.decision.as_str(),
                evaluation.reason.clone(),
            ),
        )?)?;

        if evaluation.decision != PolicyDecision::Allow {
            self.append_worker_lane_state(
                session_id,
                &lane_id,
                None,
                WorkerLaneStatus::Rejected,
                "worker lane was not allowed by policy",
            )?;

            let events = self.event_store.events_for_session(session_id)?;
            return Ok(CodexWorkerLaneResult {
                session_id,
                lane_id,
                decision: evaluation.decision,
                reason: evaluation.reason,
                observation: None,
                final_status: WorkerLaneStatus::Rejected,
                pending_commit_state: None,
                event_replay: event_replay_summary(&events),
                event_count: events.len(),
            });
        }

        let mut intent = intent;
        if matches!(
            request.workspace,
            CodexWorkerLaneWorkspace::AllocateTaskWorktree
        ) {
            let allocation = match allocate_task_worktree(&session.repo_path, &lane_id) {
                Ok(allocation) => allocation,
                Err(error) => {
                    self.append_worker_lane_state(
                        session_id,
                        &lane_id,
                        None,
                        WorkerLaneStatus::Failed,
                        &format!("task worktree allocation failed: {error}"),
                    )?;

                    let events = self.event_store.events_for_session(session_id)?;
                    return Ok(CodexWorkerLaneResult {
                        session_id,
                        lane_id,
                        decision: evaluation.decision,
                        reason: format!("task worktree allocation failed: {error}"),
                        observation: None,
                        final_status: WorkerLaneStatus::Failed,
                        pending_commit_state: None,
                        event_replay: event_replay_summary(&events),
                        event_count: events.len(),
                    });
                }
            };

            self.event_store
                .append_event(NewEvent::worker_lane_worktree_allocated(
                    session_id,
                    WorkerLaneWorktreeAllocatedPayload::new(
                        &lane_id,
                        lane_kind,
                        &session.repo_path,
                        &allocation.worktree_path,
                        &allocation.base_ref,
                    ),
                )?)?;

            intent.workspace_path = allocation.worktree_path.clone();
            intent.worktree_path = Some(allocation.worktree_path);
        }

        self.append_worker_lane_state(
            session_id,
            &lane_id,
            None,
            WorkerLaneStatus::Queued,
            "worker lane accepted by policy",
        )?;

        let previous_state = if intent.cancellation_requested {
            WorkerLaneStatus::Queued
        } else {
            self.append_worker_lane_state(
                session_id,
                &lane_id,
                Some(WorkerLaneStatus::Queued),
                WorkerLaneStatus::Running,
                &format!("{} worker lane started", request.runner.kind()),
            )?;
            WorkerLaneStatus::Running
        };

        let observation = match &request.runner {
            CodexWorkerLaneRunner::Fixture(fixture) => {
                CodexWorkerLaneFixtureAdapter.run(&intent, fixture)
            }
            CodexWorkerLaneRunner::Subprocess(subprocess) => {
                OneShotWorkerRunner.run(&intent, subprocess)
            }
        };
        self.event_store
            .append_event(NewEvent::worker_lane_observation_recorded(
                session_id,
                worker_lane_observation_payload(&lane_id, lane_kind, &observation),
            )?)?;

        self.append_worker_lane_state(
            session_id,
            &lane_id,
            Some(previous_state),
            observation.status,
            &format!("{} worker lane completed", request.runner.kind()),
        )?;

        let pending_commit_state = if observation.status == WorkerLaneStatus::Succeeded {
            self.record_worker_lane_diff_if_needed(
                session_id,
                &intent.workspace_path,
                intent.budget.max_stdout_bytes,
            )?
        } else {
            None
        };

        let events = self.event_store.events_for_session(session_id)?;
        let final_status = observation.status;

        Ok(CodexWorkerLaneResult {
            session_id,
            lane_id,
            decision: evaluation.decision,
            reason: evaluation.reason,
            observation: Some(observation),
            final_status,
            pending_commit_state,
            event_replay: event_replay_summary(&events),
            event_count: events.len(),
        })
    }

    pub fn run_codex_cli_manual_acceptance(
        &mut self,
        session_id: Uuid,
        request: CodexCliAcceptanceRequest,
    ) -> HarnessResult<CodexCliAcceptanceResult> {
        let availability = detect_codex_cli_availability(&request.availability);
        if availability.skipped_reason.is_some() {
            return Ok(CodexCliAcceptanceResult {
                session_id,
                status: CodexCliAcceptanceStatus::Skipped,
                availability,
                worker: None,
            });
        }

        let mut worker_request = CodexWorkerLaneRequest::new_subprocess(
            request.task.clone(),
            CodexWorkerSubprocess::new(
                request.codex_program,
                vec![
                    "exec".to_owned(),
                    "--sandbox".to_owned(),
                    "workspace-write".to_owned(),
                    "--json".to_owned(),
                    request.task,
                ],
            ),
        );
        worker_request.timeout_ms = request.timeout_ms;
        worker_request.budget.max_stdout_bytes = request.max_stdout_bytes;

        let worker = self.run_codex_worker_lane(session_id, worker_request)?;

        Ok(CodexCliAcceptanceResult {
            session_id,
            status: CodexCliAcceptanceStatus::Ran,
            availability,
            worker: Some(worker),
        })
    }

    fn apply_file_patch_with_events(
        &mut self,
        session_id: Uuid,
        repo_path: &str,
        patch: &FilePatchProposal,
    ) -> HarnessResult<Option<FilePatchObservation>> {
        let tool_name = harness_tools::APPLY_FILE_PATCH_TOOL.name;
        let patch_payload = file_patch_payload(patch);
        let intent = FilePatchIntent::new(
            repo_path.to_owned(),
            patch.path.clone(),
            patch.expected_content.clone(),
            patch.replacement_content.clone(),
        )?;

        self.event_store
            .append_event(NewEvent::file_patch_intended(
                session_id,
                FilePatchIntentPayload::new(tool_name, repo_path.to_owned(), patch_payload),
            )?)?;

        let evaluation = evaluate_file_patch(&patch.path, patch.replacement_content.len());
        self.event_store.append_event(NewEvent::policy_decided(
            session_id,
            PolicyDecisionPayload::new(
                tool_name,
                evaluation.decision.as_str(),
                evaluation.reason.clone(),
            ),
        )?)?;

        if evaluation.decision != PolicyDecision::Allow {
            return Ok(None);
        }

        let observation = CommandToolRuntime.run_file_patch(&intent)?;
        self.event_store
            .append_event(NewEvent::file_patch_observation_recorded(
                session_id,
                FilePatchObservationPayload::new(
                    tool_name,
                    observation.path.clone(),
                    observation.applied,
                    observation.previous_bytes,
                    observation.new_bytes,
                    observation.duration_ms,
                ),
            )?)?;

        Ok(Some(observation))
    }

    fn append_worker_lane_state(
        &mut self,
        session_id: Uuid,
        lane_id: &str,
        from_state: Option<WorkerLaneStatus>,
        to_state: WorkerLaneStatus,
        reason: &str,
    ) -> HarnessResult<()> {
        self.event_store
            .append_event(NewEvent::worker_lane_state_changed(
                session_id,
                WorkerLaneStatePayload::new(
                    lane_id,
                    "codex_cli",
                    from_state.map(|state| state.as_str().to_owned()),
                    to_state.as_str(),
                    reason,
                ),
            )?)?;

        Ok(())
    }

    fn record_worker_lane_diff_if_needed(
        &mut self,
        session_id: Uuid,
        repo_path: &str,
        max_diff_bytes: usize,
    ) -> HarnessResult<Option<String>> {
        let Some(evidence) = capture_git_diff_evidence(repo_path, max_diff_bytes)? else {
            return Ok(None);
        };

        self.event_store.append_event(NewEvent::diff_recorded(
            session_id,
            diff_summary_payload(&evidence.summary)
                .with_git_evidence(evidence.git_status, evidence.git_diff),
        )?)?;
        self.event_store
            .append_event(NewEvent::commit_approval_pending(
                session_id,
                CommitApprovalPendingPayload::new(
                    PENDING_COMMIT_APPROVAL_STATE,
                    "worker lane succeeded with reviewable diff; awaiting human commit approval",
                ),
            )?)?;

        Ok(Some(PENDING_COMMIT_APPROVAL_STATE.to_owned()))
    }

    fn finish_recovery_loop(
        &mut self,
        finish: RecoveryFinish,
    ) -> HarnessResult<SelfRecoveryLoopResult> {
        if finish.stop_reason == RecoveryStopReason::Recovered {
            self.event_store.append_event(NewEvent::diff_recorded(
                finish.session_id,
                diff_summary_payload(&finish.diff),
            )?)?;
            self.event_store
                .append_event(NewEvent::commit_approval_pending(
                    finish.session_id,
                    CommitApprovalPendingPayload::new(
                        PENDING_COMMIT_APPROVAL_STATE,
                        "verification passed after self-recovery; awaiting human commit approval",
                    ),
                )?)?;
        }

        self.event_store.append_event(NewEvent::recovery_stopped(
            finish.session_id,
            RecoveryStoppedPayload::new(
                finish.stop_reason.as_str(),
                finish.retry_count,
                finish.final_state,
            ),
        )?)?;

        let events = self.event_store.events_for_session(finish.session_id)?;
        let event_replay = event_replay_summary(&events);
        let prompt_tokens = finish.fake_turn.prompt_tokens;
        let completion_tokens = finish.fake_turn.completion_tokens;

        Ok(SelfRecoveryLoopResult {
            session_id: finish.session_id,
            initial_verification: finish.initial_verification,
            final_verification: finish.final_verification,
            report: SelfRecoveryReport {
                failure_classification: finish.failure_classification,
                recovery_plan: finish.recovery_plan,
                repair_attempts: finish.repair_attempts,
                retry_count: finish.retry_count,
                stop_reason: finish.stop_reason,
                max_recovery_rounds: finish.max_recovery_rounds,
                max_repair_bytes: finish.max_repair_bytes,
                used_repair_bytes: finish.used_repair_bytes,
            },
            diff: finish.diff,
            event_replay,
            token_ledger: TokenLedgerSummary {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
                max_output_tokens: finish.max_output_tokens,
            },
            final_state: finish.final_state.to_owned(),
            event_count: events.len(),
        })
    }
}

struct RecoveryFinish {
    session_id: Uuid,
    initial_verification: VerificationCommandResult,
    final_verification: VerificationCommandResult,
    fake_turn: FakeModelTurnResult,
    diff: DiffSummary,
    failure_classification: Option<String>,
    recovery_plan: Option<String>,
    repair_attempts: usize,
    retry_count: usize,
    stop_reason: RecoveryStopReason,
    max_recovery_rounds: usize,
    max_repair_bytes: usize,
    used_repair_bytes: usize,
    max_output_tokens: usize,
    final_state: &'static str,
}

pub fn apply_database_migrations(database_url: &str) -> HarnessResult<()> {
    let store = PostgresEventStore::connect(database_url)?;
    store.apply_migrations()
}

pub fn project_session(
    session_id: Uuid,
    events: &[EventEnvelope],
) -> HarnessResult<SessionProjection> {
    let Some(first_event) = events.first() else {
        return Err(HarnessError::new(format!(
            "session not found: {session_id}"
        )));
    };

    if first_event.event_type != EventType::SessionStarted {
        return Err(HarnessError::new(
            "session replay must start with session.started",
        ));
    }

    let payload: SessionStartedPayload = serde_json::from_value(first_event.payload.clone())
        .map_err(|error| HarnessError::new(error.to_string()))?;

    Ok(SessionProjection {
        session_id,
        repo_path: payload.repo_path,
        status: SessionStatus::Started,
        event_count: events.len(),
    })
}

fn task_projections_from_events(
    session_id: Uuid,
    events: &[EventEnvelope],
) -> HarnessResult<Vec<TaskProjection>> {
    let mut tasks = Vec::new();

    for event in events {
        if event.event_type == EventType::TaskCreated {
            let payload: TaskCreatedPayload = serde_json::from_value(event.payload.clone())
                .map_err(|error| HarnessError::new(error.to_string()))?;
            tasks.push(task_projection_from_created_payload(
                session_id,
                payload,
                events.len(),
            ));
            continue;
        }

        let Some(task_id) = payload_task_id(&event.payload)? else {
            continue;
        };
        let Some(task) = tasks.iter_mut().find(|task| task.task_id == task_id) else {
            continue;
        };

        match event.event_type {
            EventType::WorkerLaneRequested => {
                task.worker_lane_kind = payload_string(&event.payload, "lane_kind")?;
                task.input = payload_string(&event.payload, "task")?;
            }
            EventType::WorkerLaneStateChanged => {
                task.status = payload_string(&event.payload, "to_state")?;
            }
            EventType::WorkerLaneWorktreeAllocated => {
                task.worktree = Some(TaskWorktreeProjection {
                    lane_id: payload_string(&event.payload, "lane_id")?,
                    lane_kind: payload_string(&event.payload, "lane_kind")?,
                    worktree_path: payload_string(&event.payload, "worktree_path")?,
                    base_ref: payload_string(&event.payload, "base_ref")?,
                });
            }
            EventType::WorkerLaneObservationRecorded => {
                let status = payload_string(&event.payload, "status")?;
                task.status = status.clone();
                task.worker_output = Some(TaskWorkerOutputProjection {
                    lane_id: payload_string(&event.payload, "lane_id")?,
                    lane_kind: payload_string(&event.payload, "lane_kind")?,
                    status,
                    exit_code: payload_i32_optional(&event.payload, "exit_code")?,
                    stdout: payload_string(&event.payload, "stdout")?,
                    stderr: payload_string(&event.payload, "stderr")?,
                    duration_ms: payload_u64(&event.payload, "duration_ms")?,
                });
            }
            EventType::DiffRecorded => {
                task.diff = Some(diff_summary_from_payload(&event.payload)?);
            }
            EventType::CommitApprovalPending => {
                let state = payload_string(&event.payload, "state")?;
                task.status = state.clone();
                task.approval = Some(TaskApprovalProjection {
                    state,
                    summary: payload_string(&event.payload, "summary")?,
                    rejection_reason: None,
                });
            }
            EventType::CommitApproved | EventType::CommitRejected => {
                let state = payload_string(&event.payload, "state")?;
                let reason = payload_string(&event.payload, "reason")?;
                task.status = state.clone();
                task.approval = Some(TaskApprovalProjection {
                    rejection_reason: (event.event_type == EventType::CommitRejected)
                        .then(|| reason.clone()),
                    state,
                    summary: reason,
                });
            }
            EventType::CommitStarted | EventType::CommitSucceeded | EventType::CommitFailed => {
                let state = payload_string(&event.payload, "state")?;
                task.status = state.clone();
                task.commit = Some(TaskCommitProjection {
                    state,
                    message: payload_string(&event.payload, "message")?,
                    commit_sha: payload_optional_string(&event.payload, "commit_sha"),
                    failure_reason: payload_optional_string(&event.payload, "failure_reason"),
                });
            }
            _ => {}
        }
    }

    Ok(tasks)
}

fn task_projection_from_created_payload(
    session_id: Uuid,
    payload: TaskCreatedPayload,
    event_count: usize,
) -> TaskProjection {
    TaskProjection {
        session_id,
        task_id: payload.task_id,
        repo_path: payload.repo_path,
        input: payload.input,
        status: payload.status,
        worker_lane_kind: payload.worker_lane_kind,
        context_budget: ContextBudget {
            max_bytes: payload.max_context_bytes,
            max_files: payload.max_context_files,
            max_skill_files: payload.max_context_skill_files,
        },
        focus_terms: payload.focus_terms,
        max_output_tokens: payload.max_output_tokens,
        worktree: None,
        worker_output: None,
        diff: None,
        approval: None,
        commit: None,
        event_count,
    }
}

fn context_compiled_payload(bundle: &CompiledContextBundle) -> ContextCompiledPayload {
    ContextCompiledPayload {
        repo_path: bundle.repo_path.clone(),
        budget_bytes: bundle.budget.max_bytes,
        budget_files: bundle.budget.max_files,
        budget_skill_files: bundle.budget.max_skill_files,
        used_bytes: bundle.used_bytes,
        truncated: bundle.truncated,
        sources: bundle
            .sources
            .iter()
            .map(|source| {
                ContextSourcePayload::new(
                    source.kind.as_str(),
                    source.path.clone(),
                    source.content.clone(),
                    source.original_bytes,
                    source.included_bytes,
                    source.truncated,
                )
            })
            .collect(),
        skills: bundle
            .skills
            .iter()
            .map(|skill| {
                SkillMetadataPayload::new(
                    skill.path.clone(),
                    skill.name.clone(),
                    skill.description.clone(),
                )
            })
            .collect(),
    }
}

fn file_patch_payload(patch: &FilePatchProposal) -> FilePatchPayload {
    FilePatchPayload::new(
        patch.path.clone(),
        patch.expected_content.clone(),
        patch.replacement_content.clone(),
    )
}

fn classify_recovery_failure(observation: &CommandObservation) -> String {
    let output = format!("{}\n{}", observation.stdout, observation.stderr);

    if output.contains("missing recovery marker")
        || output.contains("fake_patch_contains_recovery_marker")
    {
        "fixture_missing_recovery_marker".to_owned()
    } else {
        "verification_failed".to_owned()
    }
}

fn recovery_repair_patch(
    repo_path: &str,
    patch: &FilePatchProposal,
) -> HarnessResult<FilePatchProposal> {
    let path = Path::new(repo_path).join(&patch.path);
    let current_content =
        fs::read_to_string(path).map_err(|error| HarnessError::new(error.to_string()))?;
    let replacement_content = if current_content.contains("recovered=true") {
        current_content.clone()
    } else {
        format!("{current_content}recovered=true\n")
    };

    Ok(FilePatchProposal {
        path: patch.path.clone(),
        expected_content: Some(current_content),
        replacement_content,
    })
}

fn diff_summary_from_patch(patch: &FilePatchProposal) -> DiffSummary {
    DiffSummary {
        files_changed: 1,
        insertions: line_count(&patch.replacement_content),
        deletions: patch.expected_content.as_deref().map_or(0, line_count),
        paths: vec![patch.path.clone()],
    }
}

fn approval_projection_from_events(
    session_id: Uuid,
    events: &[EventEnvelope],
) -> HarnessResult<ApprovalProjection> {
    let mut diff = None;
    let mut state = None;
    let mut summary = String::new();
    let mut rejection_reason = None;

    for event in events {
        match event.event_type {
            EventType::DiffRecorded => {
                diff = Some(diff_summary_from_payload(&event.payload)?);
            }
            EventType::CommitApprovalPending => {
                state = Some(ApprovalState::Pending);
                summary = event
                    .payload
                    .get("summary")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_owned();
                rejection_reason = None;
            }
            EventType::CommitApproved => {
                state = Some(ApprovalState::Approved);
                summary = event
                    .payload
                    .get("reason")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_owned();
                rejection_reason = None;
            }
            EventType::CommitRejected => {
                state = Some(ApprovalState::Rejected);
                let reason = event
                    .payload
                    .get("reason")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_owned();
                summary = reason.clone();
                rejection_reason = Some(reason);
            }
            _ => {}
        }
    }

    let diff = diff.ok_or_else(|| HarnessError::new("pending diff was not recorded"))?;
    let state = state.ok_or_else(|| HarnessError::new("approval state was not recorded"))?;

    Ok(ApprovalProjection {
        session_id,
        state: state.as_str().to_owned(),
        diff,
        summary,
        rejection_reason,
        event_count: events.len(),
    })
}

fn commit_handoff_projection_from_events(
    session_id: Uuid,
    events: &[EventEnvelope],
) -> HarnessResult<CommitHandoffProjection> {
    let mut projection = None;

    for event in events {
        match event.event_type {
            EventType::CommitStarted | EventType::CommitSucceeded | EventType::CommitFailed => {
                projection = Some(CommitHandoffProjection {
                    session_id,
                    state: payload_string(&event.payload, "state")?,
                    repo_path: payload_string(&event.payload, "repo_path")?,
                    message: payload_string(&event.payload, "message")?,
                    commit_sha: event
                        .payload
                        .get("commit_sha")
                        .and_then(|value| value.as_str())
                        .map(ToOwned::to_owned),
                    failure_reason: event
                        .payload
                        .get("failure_reason")
                        .and_then(|value| value.as_str())
                        .map(ToOwned::to_owned),
                    event_count: events.len(),
                });
            }
            _ => {}
        }
    }

    projection.ok_or_else(|| HarnessError::new("commit handoff was not recorded"))
}

fn existing_commit_handoff_state_from_events(
    events: &[EventEnvelope],
) -> HarnessResult<Option<String>> {
    let mut state = None;

    for event in events {
        match event.event_type {
            EventType::CommitStarted | EventType::CommitSucceeded | EventType::CommitFailed => {
                state = Some(payload_string(&event.payload, "state")?);
            }
            _ => {}
        }
    }

    Ok(state)
}

fn commit_repo_path_from_events(events: &[EventEnvelope]) -> HarnessResult<String> {
    let mut session_repo_path = None;
    let mut latest_worktree_path = None;
    let mut diff_repo_path = None;

    for event in events {
        match event.event_type {
            EventType::SessionStarted => {
                session_repo_path = Some(payload_string(&event.payload, "repo_path")?);
            }
            EventType::WorkerLaneWorktreeAllocated => {
                latest_worktree_path = Some(payload_string(&event.payload, "worktree_path")?);
            }
            EventType::DiffRecorded => {
                diff_repo_path = latest_worktree_path
                    .clone()
                    .or_else(|| session_repo_path.clone());
            }
            _ => {}
        }
    }

    diff_repo_path.ok_or_else(|| HarnessError::new("diff repository path was not recorded"))
}

fn payload_string(payload: &serde_json::Value, key: &str) -> HarnessResult<String> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| HarnessError::new(format!("payload {key} is missing")))
}

fn payload_optional_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn payload_task_id(payload: &serde_json::Value) -> HarnessResult<Option<Uuid>> {
    payload
        .get("task_id")
        .and_then(|value| value.as_str())
        .map(|value| Uuid::parse_str(value).map_err(|error| HarnessError::new(error.to_string())))
        .transpose()
}

fn payload_i32_optional(payload: &serde_json::Value, key: &str) -> HarnessResult<Option<i32>> {
    payload
        .get(key)
        .and_then(|value| value.as_i64())
        .map(|value| {
            i32::try_from(value)
                .map_err(|_| HarnessError::new(format!("payload {key} is out of range")))
        })
        .transpose()
}

fn payload_u64(payload: &serde_json::Value, key: &str) -> HarnessResult<u64> {
    payload
        .get(key)
        .and_then(|value| value.as_u64())
        .ok_or_else(|| HarnessError::new(format!("payload {key} is missing")))
}

fn diff_summary_from_payload(payload: &serde_json::Value) -> HarnessResult<DiffSummary> {
    let files_changed = payload_usize(payload, "files_changed")?;
    let insertions = payload_usize(payload, "insertions")?;
    let deletions = payload_usize(payload, "deletions")?;
    let paths = payload
        .get("paths")
        .and_then(|value| value.as_array())
        .ok_or_else(|| HarnessError::new("diff paths are missing"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| HarnessError::new("diff path is not a string"))
        })
        .collect::<HarnessResult<Vec<_>>>()?;

    Ok(DiffSummary {
        files_changed,
        insertions,
        deletions,
        paths,
    })
}

fn payload_usize(payload: &serde_json::Value, key: &str) -> HarnessResult<usize> {
    let value = payload
        .get(key)
        .and_then(|value| value.as_u64())
        .ok_or_else(|| HarnessError::new(format!("diff {key} is missing")))?;

    usize::try_from(value).map_err(|_| HarnessError::new(format!("diff {key} is out of range")))
}

#[must_use]
pub fn detect_codex_cli_availability(
    request: &CodexCliAvailabilityRequest,
) -> CodexCliAvailability {
    let output = Command::new(&request.program).arg("--version").output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return CodexCliAvailability {
                program: request.program.clone(),
                available: false,
                authenticated: false,
                version: None,
                skipped_reason: Some(format!(
                    "Codex CLI executable could not be started: {error}"
                )),
            };
        }
    };

    if !output.status.success() {
        return CodexCliAvailability {
            program: request.program.clone(),
            available: false,
            authenticated: false,
            version: None,
            skipped_reason: Some(format!(
                "Codex CLI version check failed with status {}: {}{}",
                output
                    .status
                    .code()
                    .map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )),
        };
    }

    let version = version_from_output(&output.stdout, &output.stderr);
    let authenticated = request.codex_api_key_present
        || request
            .codex_home
            .as_ref()
            .is_some_and(|codex_home| codex_home.join("auth.json").exists());
    let skipped_reason = (!authenticated).then_some(
        "Codex CLI authentication was not detected; set CODEX_API_KEY or run codex login"
            .to_owned(),
    );

    CodexCliAvailability {
        program: request.program.clone(),
        available: true,
        authenticated,
        version,
        skipped_reason,
    }
}

fn version_from_output(stdout: &[u8], stderr: &[u8]) -> Option<String> {
    let output = if stdout.is_empty() { stderr } else { stdout };
    let version = String::from_utf8_lossy(output).trim().to_owned();

    (!version.is_empty()).then_some(version)
}

fn default_codex_home() -> Option<PathBuf> {
    env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .ok()
        .map(|home| PathBuf::from(home).join(".codex"))
}

fn capture_git_diff_evidence(
    repo_path: &str,
    max_diff_bytes: usize,
) -> HarnessResult<Option<GitDiffEvidence>> {
    let git_status = run_git_text(repo_path, &["status", "--short"])?;
    if git_status.trim().is_empty() {
        return Ok(None);
    }

    let numstat = run_git_text(repo_path, &["diff", "--numstat"])?;
    let git_diff = truncate_to_bytes(&run_git_text(repo_path, &["diff", "--"])?, max_diff_bytes);
    let summary = diff_summary_from_git(&numstat, &git_status);

    Ok(Some(GitDiffEvidence {
        summary,
        git_status,
        git_diff,
    }))
}

fn run_harness_commit(repo_path: &str, message: &str) -> HarnessResult<String> {
    let status = run_git_text(repo_path, &["status", "--short"])?;
    if status.trim().is_empty() {
        return Err(HarnessError::new(
            "approved diff has no local changes to commit",
        ));
    }

    run_git_text(repo_path, &["add", "-A"])?;
    run_git_commit(repo_path, message)?;
    Ok(run_git_text(repo_path, &["rev-parse", "HEAD"])?
        .trim()
        .to_owned())
}

fn run_git_commit(repo_path: &str, message: &str) -> HarnessResult<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("-c")
        .arg("user.name=Coding Agent Harness")
        .arg("-c")
        .arg("user.email=harness@example.invalid")
        .arg("commit")
        .arg("-m")
        .arg(message)
        .output()
        .map_err(|error| HarnessError::new(format!("could not start git commit: {error}")))?;

    if !output.status.success() {
        return Err(HarnessError::new(format!(
            "git commit failed with status {}: {}{}",
            output
                .status
                .code()
                .map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(())
}

fn run_git_text(repo_path: &str, args: &[&str]) -> HarnessResult<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .output()
        .map_err(|error| HarnessError::new(format!("could not start git {:?}: {error}", args)))?;

    if !output.status.success() {
        return Err(HarnessError::new(format!(
            "git {:?} failed with status {}: {}{}",
            args,
            output
                .status
                .code()
                .map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn diff_summary_from_git(numstat: &str, git_status: &str) -> DiffSummary {
    let mut insertions = 0;
    let mut deletions = 0;
    let mut paths = Vec::new();

    for line in numstat.lines() {
        let mut parts = line.split('\t');
        let added = parts.next();
        let removed = parts.next();
        let path = parts.next();

        let Some(path) = path else {
            continue;
        };

        insertions += added.and_then(parse_numstat_count).unwrap_or(0);
        deletions += removed.and_then(parse_numstat_count).unwrap_or(0);
        paths.push(path.to_owned());
    }

    if paths.is_empty() {
        paths.extend(git_status.lines().filter_map(status_path));
    }

    paths.sort();
    paths.dedup();

    DiffSummary {
        files_changed: paths.len(),
        insertions,
        deletions,
        paths,
    }
}

fn parse_numstat_count(value: &str) -> Option<usize> {
    value.parse::<usize>().ok()
}

fn status_path(line: &str) -> Option<String> {
    line.get(3..)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
}

fn diff_summary_payload(diff: &DiffSummary) -> DiffSummaryPayload {
    DiffSummaryPayload::new(
        diff.files_changed,
        diff.insertions,
        diff.deletions,
        diff.paths.clone(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedWorkerWorkspace {
    workspace_path: String,
    worktree_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskWorktreeAllocation {
    worktree_path: String,
    base_ref: String,
}

fn requested_codex_worker_workspace(
    workspace: &CodexWorkerLaneWorkspace,
    session_repo_path: &str,
) -> ResolvedWorkerWorkspace {
    match workspace {
        CodexWorkerLaneWorkspace::AllocateTaskWorktree
        | CodexWorkerLaneWorkspace::DangerousCurrentWorkspace => ResolvedWorkerWorkspace {
            workspace_path: session_repo_path.to_owned(),
            worktree_path: None,
        },
        CodexWorkerLaneWorkspace::ExistingWorktree { path } => ResolvedWorkerWorkspace {
            workspace_path: path.clone(),
            worktree_path: Some(path.clone()),
        },
    }
}

fn allocate_task_worktree(
    session_repo_path: &str,
    lane_id: &str,
) -> HarnessResult<TaskWorktreeAllocation> {
    let repo_path = Path::new(session_repo_path);
    let repo_name = repo_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("repo");
    let parent = repo_path.parent().unwrap_or_else(|| Path::new("."));
    let worktree_path = parent
        .join(".harness-worktrees")
        .join(format!("{repo_name}-{lane_id}"));

    if let Some(parent) = worktree_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            HarnessError::new(format!(
                "could not create task worktree parent {}: {error}",
                parent.display()
            ))
        })?;
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("worktree")
        .arg("add")
        .arg("--detach")
        .arg(&worktree_path)
        .arg("HEAD")
        .output()
        .map_err(|error| HarnessError::new(format!("could not start git worktree add: {error}")))?;

    if !output.status.success() {
        return Err(HarnessError::new(format!(
            "git worktree add failed with status {}: {}{}",
            output
                .status
                .code()
                .map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(TaskWorktreeAllocation {
        worktree_path: canonicalize_if_possible(&worktree_path),
        base_ref: "HEAD".to_owned(),
    })
}

fn canonicalize_if_possible(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| PathBuf::from(path));

    path.display().to_string()
}

fn truncate_to_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }

    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }

    value[..end].to_owned()
}

fn worker_lane_request_payload(
    lane_id: &str,
    lane_kind: &str,
    intent: &CodexWorkerLaneIntent,
) -> WorkerLaneRequestPayload {
    WorkerLaneRequestPayload {
        task_id: None,
        lane_id: lane_id.to_owned(),
        lane_kind: lane_kind.to_owned(),
        task: intent.task.clone(),
        workspace_path: intent.workspace_path.clone(),
        worktree_path: intent.worktree_path.clone(),
        timeout_ms: intent.timeout_ms,
        cancellation_requested: intent.cancellation_requested,
        budget: WorkerLaneBudgetPayload::new(
            intent.budget.max_prompt_tokens,
            intent.budget.max_output_tokens,
            intent.budget.max_stdout_bytes,
        ),
    }
}

fn worker_lane_observation_payload(
    lane_id: &str,
    lane_kind: &str,
    observation: &CodexWorkerLaneObservation,
) -> WorkerLaneObservationPayload {
    WorkerLaneObservationPayload {
        task_id: None,
        lane_id: lane_id.to_owned(),
        lane_kind: lane_kind.to_owned(),
        status: observation.status.as_str().to_owned(),
        exit_code: observation.exit_code,
        stdout: observation.stdout.clone(),
        stderr: observation.stderr.clone(),
        duration_ms: observation.duration_ms,
        prompt_tokens: observation.usage.prompt_tokens,
        completion_tokens: observation.usage.completion_tokens,
        usage_confidence: observation.usage.confidence.as_str().to_owned(),
    }
}

fn event_replay_summary(events: &[EventEnvelope]) -> EventReplaySummary {
    EventReplaySummary {
        total_events: events.len(),
        last_event_type: events
            .last()
            .map(|event| event.event_type.as_str().to_owned()),
        event_types: events
            .iter()
            .map(|event| event.event_type.as_str().to_owned())
            .collect(),
    }
}

fn line_count(content: &str) -> usize {
    content.lines().count()
}

#[must_use]
pub fn doctor_report() -> String {
    let context_inputs = harness_context::bootstrap_context_inputs().join(", ");

    format!(
        "{} bootstrap\n- database env: {}\n- migrations: {}\n- eventlog schema: v{}\n- context inputs: {}\n- verification tool: {}",
        harness_core::PRODUCT_NAME,
        harness_db::DATABASE_URL_ENV,
        harness_db::MIGRATIONS_DIR,
        harness_events::eventlog_schema_version(),
        context_inputs,
        harness_tools::VERIFY_COMMAND_TOOL.name
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_report_mentions_runtime_baseline() {
        let report = doctor_report();

        assert!(report.contains("Coding Agent Harness bootstrap"));
        assert!(report.contains("HARNESS_DATABASE_URL"));
        assert!(report.to_ascii_lowercase().contains("eventlog"));
    }

    #[test]
    fn project_session_replays_started_session() {
        let session_id = Uuid::new_v4();
        let event = NewEvent::session_started(session_id, SessionStartedPayload::new("C:/repo"))
            .expect("session started event");
        let envelope = EventEnvelope {
            event_id: event.event_id,
            session_id,
            sequence: 1,
            event_type: event.event_type,
            schema_version: event.schema_version,
            payload: event.payload,
        };

        let projection = project_session(session_id, &[envelope]).expect("projection");

        assert_eq!(projection.session_id, session_id);
        assert_eq!(projection.repo_path, "C:/repo");
        assert_eq!(projection.status, SessionStatus::Started);
        assert_eq!(projection.event_count, 1);
    }

    #[test]
    fn project_session_keeps_count_after_tool_events() {
        let session_id = Uuid::new_v4();
        let session_started =
            NewEvent::session_started(session_id, SessionStartedPayload::new("C:/repo"))
                .expect("session started event");
        let tool_intent = NewEvent::tool_call_intended(
            session_id,
            ToolCallIntentPayload::new(
                "verify_command",
                "cargo",
                vec!["--version".to_owned()],
                "C:/repo",
            ),
        )
        .expect("tool intent event");
        let events = vec![
            EventEnvelope {
                event_id: session_started.event_id,
                session_id,
                sequence: 1,
                event_type: session_started.event_type,
                schema_version: session_started.schema_version,
                payload: session_started.payload,
            },
            EventEnvelope {
                event_id: tool_intent.event_id,
                session_id,
                sequence: 2,
                event_type: tool_intent.event_type,
                schema_version: tool_intent.schema_version,
                payload: tool_intent.payload,
            },
        ];

        let projection = project_session(session_id, &events).expect("projection");

        assert_eq!(projection.event_count, 2);
    }

    #[test]
    fn task_projection_replays_task_scoped_events_without_database() {
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let session_started =
            NewEvent::session_started(session_id, SessionStartedPayload::new("C:/repo"))
                .expect("session started event");
        let task_created = NewEvent::task_created(
            session_id,
            TaskCreatedPayload::new(
                task_id,
                "C:/repo",
                "draft a task patch",
                TASK_CREATED_STATE,
                DEFAULT_TASK_WORKER_LANE_KIND,
                4096,
                8,
                2,
                vec!["agent".to_owned()],
                256,
            ),
        )
        .expect("task created event");
        let diff = NewEvent::diff_recorded(
            session_id,
            DiffSummaryPayload::new(1, 3, 0, vec![".harness/task.md".to_owned()])
                .with_task_id(task_id),
        )
        .expect("diff event");
        let approval = NewEvent::commit_approval_pending(
            session_id,
            CommitApprovalPendingPayload::new(
                PENDING_COMMIT_APPROVAL_STATE,
                "verification passed; awaiting human commit approval",
            )
            .with_task_id(task_id),
        )
        .expect("approval event");
        let commit = NewEvent::commit_succeeded(
            session_id,
            CommitHandoffPayload::succeeded(
                "C:/repo",
                "Commit approved task patch",
                "runtime",
                "0123456789abcdef0123456789abcdef01234567",
            )
            .with_task_id(task_id),
        )
        .expect("commit event");
        let events = vec![
            event_envelope(1, session_started),
            event_envelope(2, task_created),
            event_envelope(3, diff),
            event_envelope(4, approval),
            event_envelope(5, commit),
        ];

        let tasks = task_projections_from_events(session_id, &events).expect("task projection");

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_id, task_id);
        assert_eq!(tasks[0].status, COMMITTED_STATE);
        assert_eq!(tasks[0].input, "draft a task patch");
        assert_eq!(tasks[0].worker_lane_kind, DEFAULT_TASK_WORKER_LANE_KIND);
        assert_eq!(tasks[0].context_budget.max_bytes, 4096);
        assert_eq!(tasks[0].focus_terms, vec!["agent"]);
        assert_eq!(
            tasks[0].diff.as_ref().expect("diff slot").paths,
            vec![".harness/task.md"]
        );
        assert_eq!(
            tasks[0].approval.as_ref().expect("approval slot").state,
            PENDING_COMMIT_APPROVAL_STATE
        );
        assert_eq!(
            tasks[0]
                .commit
                .as_ref()
                .expect("commit slot")
                .commit_sha
                .as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(tasks[0].event_count, 5);
    }

    #[test]
    fn context_payload_preserves_sources_and_skill_metadata() {
        let bundle = CompiledContextBundle {
            repo_path: "C:/repo".to_owned(),
            budget: ContextBudget {
                max_bytes: 4096,
                max_files: 8,
                max_skill_files: 8,
            },
            used_bytes: 12,
            truncated: false,
            sources: vec![harness_context::ContextSource {
                kind: harness_context::ContextSourceKind::RepositoryInstructions,
                path: "AGENTS.md".to_owned(),
                content: "instructions".to_owned(),
                original_bytes: 12,
                included_bytes: 12,
                truncated: false,
            }],
            skills: vec![harness_context::SkillMetadata {
                path: ".codex/skills/demo/SKILL.md".to_owned(),
                name: Some("demo".to_owned()),
                description: Some("Demo skill".to_owned()),
            }],
        };

        let payload = context_compiled_payload(&bundle);

        assert_eq!(payload.budget_bytes, 4096);
        assert_eq!(payload.budget_files, 8);
        assert_eq!(payload.budget_skill_files, 8);
        assert_eq!(payload.sources[0].path, "AGENTS.md");
        assert_eq!(payload.skills[0].name.as_deref(), Some("demo"));
    }

    fn event_envelope(sequence: i64, event: NewEvent) -> EventEnvelope {
        EventEnvelope {
            event_id: event.event_id,
            session_id: event.session_id,
            sequence,
            event_type: event.event_type,
            schema_version: event.schema_version,
            payload: event.payload,
        }
    }
}
