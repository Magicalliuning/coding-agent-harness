use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};

pub use harness_context::ContextBudget;
use harness_context::{CompiledContextBundle, ContextCompileRequest, compile_repository_context};
use harness_core::{HarnessError, HarnessResult};
use harness_db::{PostgresEventStore, TaskLeaseExpirationRecord, TaskQueueRecord};
use harness_events::{
    CommitApprovalDecisionPayload, CommitApprovalPendingPayload, CommitHandoffPayload,
    ContextCompiledPayload, ContextSourcePayload, DiffSummaryPayload, EventEnvelope, EventType,
    FilePatchIntentPayload, FilePatchObservationPayload, FilePatchPayload, ModelDecisionPayload,
    ModelProviderErrorPayload, ModelProviderSkippedPayload, ModelRequestPayload, NewEvent,
    PolicyDecisionPayload, RecoveryFailurePayload, RecoveryPlanPayload,
    RecoveryRepairAttemptPayload, RecoveryStoppedPayload, SessionStartedPayload,
    SkillMetadataPayload, TaskCreatedPayload, TaskLeasePayload, TaskQueuePayload,
    ToolCallIntentPayload, ToolObservationPayload, WorkerLaneBudgetPayload,
    WorkerLaneObservationPayload, WorkerLaneRequestPayload, WorkerLaneStatePayload,
    WorkerLaneWorktreeAllocatedPayload,
};
use harness_models::{
    DETERMINISTIC_FAKE_MODEL_ID, DETERMINISTIC_FAKE_PROVIDER_KIND, FilePatchProposal,
    ModelProviderOutcome, ModelProviderRequest, ModelProviderRouter, ModelProviderSelection,
    OPENAI_COMPATIBLE_PROVIDER_KIND,
};
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
use serde::Serialize;
use uuid::Uuid;

pub const PENDING_COMMIT_APPROVAL_STATE: &str = "pending_commit_approval";
pub const COMMIT_APPROVED_STATE: &str = "approved";
pub const COMMIT_REJECTED_STATE: &str = "rejected";
pub const COMMITTING_STATE: &str = "committing";
pub const COMMITTED_STATE: &str = "committed";
pub const COMMIT_FAILED_STATE: &str = "commit_failed";
pub const TASK_CREATED_STATE: &str = "created";
pub const TASK_QUEUED_STATE: &str = "queued";
pub const TASK_LEASED_STATE: &str = "leased";
pub const TASK_LEASE_EXPIRED_STATE: &str = "expired";
pub const TASK_RETRY_QUEUED_STATE: &str = "retry_queued";
pub const TASK_COMPLETED_STATE: &str = "completed";
pub const TASK_FAILED_STATE: &str = "failed";
pub const TASK_CANCELLED_STATE: &str = "cancelled";
pub const TASK_STOPPED_STATE: &str = "stopped";
pub const TASK_STOP_REASON_LEASE_EXPIRED: &str = "lease_expired";
pub const TASK_STOP_REASON_MAX_RETRIES_EXCEEDED: &str = "max_retries_exceeded";
pub const DEFAULT_TASK_WORKER_LANE_KIND: &str = "codex_cli_worker_lane";
pub const QUEUED_WORKER_LEASE_TIMEOUT_MARGIN_MS: u64 = 1_000;
pub const RECOVERY_FAILED_STATE: &str = "recovery_failed";
pub const SELF_RECOVERY_MAX_ROUNDS: usize = 2;
pub const DEFAULT_CODEX_CLI_PROGRAM: &str = "codex";
pub const DEFAULT_CODEX_CLI_ACCEPTANCE_TIMEOUT_MS: u64 = 120_000;
pub const DEFAULT_CODEX_CLI_ACCEPTANCE_TASK: &str = "Create or update .harness/codex-manual-acceptance.md with a short note saying local Codex CLI manual acceptance ran successfully. Do not commit the change.";
pub const DEFAULT_TIMELINE_PAYLOAD_LIMIT_BYTES: usize = 512;
pub const DEFAULT_OPENAI_COMPATIBLE_API_KEY_ENV: &str = "OPENAI_API_KEY";

pub struct Runtime {
    event_store: PostgresEventStore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeReportSection {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub key_points: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeReportNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub summary: String,
    pub glossary_terms: Vec<String>,
    pub adr_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeReportEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub label: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeReportNonGoal {
    pub id: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeArchitectureReport {
    pub report_id: String,
    pub title: String,
    pub summary: String,
    pub boundary_adrs: Vec<String>,
    pub sections: Vec<RuntimeReportSection>,
    pub nodes: Vec<RuntimeReportNode>,
    pub edges: Vec<RuntimeReportEdge>,
    pub non_goals: Vec<RuntimeReportNonGoal>,
    pub source_of_truth: String,
    pub projection_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeDataFlowStep {
    pub id: String,
    pub order: usize,
    pub label: String,
    pub component_id: String,
    pub event_types: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeDataFlowReport {
    pub report_id: String,
    pub title: String,
    pub summary: String,
    pub boundary_adrs: Vec<String>,
    pub steps: Vec<RuntimeDataFlowStep>,
    pub edges: Vec<RuntimeReportEdge>,
    pub non_goals: Vec<RuntimeReportNonGoal>,
    pub source_of_truth: String,
    pub projection_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeStorageTableReport {
    pub id: String,
    pub table_name: String,
    pub role: String,
    pub ownership_boundary: String,
    pub key_fields: Vec<String>,
    pub derived_from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeStorageProjectionReport {
    pub id: String,
    pub name: String,
    pub source: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeStorageReport {
    pub report_id: String,
    pub title: String,
    pub summary: String,
    pub boundary_adrs: Vec<String>,
    pub sections: Vec<RuntimeReportSection>,
    pub tables: Vec<RuntimeStorageTableReport>,
    pub projections: Vec<RuntimeStorageProjectionReport>,
    pub non_goals: Vec<RuntimeReportNonGoal>,
    pub source_of_truth: String,
    pub projection_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityNode {
    pub id: String,
    pub name: String,
    pub phase: String,
    pub status: String,
    pub avp_status: String,
    pub problem: String,
    pub user_visible_result: String,
    pub current_gap: String,
    pub dependencies: Vec<String>,
    pub operation_entries: Vec<String>,
    pub acceptance_gates: Vec<String>,
    pub runtime_boundaries: Vec<String>,
    pub non_goals: Vec<String>,
    pub next_owner_gate: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityRegistryReport {
    pub report_id: String,
    pub title: String,
    pub summary: String,
    pub capability_count: usize,
    pub capabilities: Vec<CapabilityNode>,
    pub source_of_truth: String,
    pub projection_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityGateReport {
    pub capability_id: String,
    pub capability_name: String,
    pub phase: String,
    pub status: String,
    pub avp_status: String,
    pub gate_status: String,
    pub ready_for_implementation: bool,
    pub ready_for_merge: bool,
    pub blocking_items: Vec<String>,
    pub required_evidence: Vec<String>,
    pub source_of_truth: String,
    pub projection_kind: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionTaskStatusCount {
    pub status: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionInspectReport {
    pub session_id: Uuid,
    pub repo_path: String,
    pub status: String,
    pub event_count: usize,
    pub latest_event_sequence: Option<i64>,
    pub latest_event_type: Option<String>,
    pub task_count: usize,
    pub task_status_counts: Vec<SessionTaskStatusCount>,
    pub source_of_truth: String,
    pub projection_kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventTimelineInspectRequest {
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub payload_limit_bytes: usize,
}

impl EventTimelineInspectRequest {
    #[must_use]
    pub const fn new(session_id: Uuid) -> Self {
        Self {
            session_id,
            task_id: None,
            payload_limit_bytes: DEFAULT_TIMELINE_PAYLOAD_LIMIT_BYTES,
        }
    }

    #[must_use]
    pub const fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    #[must_use]
    pub const fn with_payload_limit_bytes(mut self, payload_limit_bytes: usize) -> Self {
        self.payload_limit_bytes = payload_limit_bytes;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoundedPayloadSummary {
    pub text: String,
    pub original_bytes: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventTimelineEntry {
    pub event_id: Uuid,
    pub session_id: Uuid,
    pub sequence: i64,
    pub event_type: String,
    pub schema_version: u16,
    pub task_id: Option<Uuid>,
    pub payload: BoundedPayloadSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventTimelineReport {
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub event_count: usize,
    pub payload_limit_bytes: usize,
    pub events: Vec<EventTimelineEntry>,
    pub source_of_truth: String,
    pub projection_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskContextBudgetReport {
    pub max_bytes: usize,
    pub max_files: usize,
    pub max_skill_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskInspectQueueReport {
    pub status: String,
    pub worker_id: Option<String>,
    pub lease_id: Option<Uuid>,
    pub lease_deadline_ms: Option<i64>,
    pub retry_count: Option<i32>,
    pub max_retries: Option<i32>,
    pub stop_reason: Option<String>,
    pub last_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskInspectLeaseReport {
    pub lease_id: Uuid,
    pub worker_id: String,
    pub status: String,
    pub lease_deadline_ms: Option<i64>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskInspectApprovalReport {
    pub state: String,
    pub summary: String,
    pub actor: Option<String>,
    pub reason: Option<String>,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskInspectCommitReport {
    pub state: String,
    pub repo_path: Option<String>,
    pub message: String,
    pub actor: Option<String>,
    pub commit_sha: Option<String>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskInspectWorkspaceReport {
    pub worker_workspace_path: Option<String>,
    pub worker_worktree_path: Option<String>,
    pub allocated_worktree_path: Option<String>,
    pub session_repo_path: Option<String>,
    pub diff_repo_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskInspectDiffReport {
    pub repo_path: Option<String>,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskInspectEventSummary {
    pub event_count: usize,
    pub first_sequence: Option<i64>,
    pub latest_sequence: Option<i64>,
    pub latest_event_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskInspectReport {
    pub session_id: Uuid,
    pub task_id: Uuid,
    pub repo_path: String,
    pub status: String,
    pub input_summary: BoundedPayloadSummary,
    pub worker_lane_kind: String,
    pub context_budget: TaskContextBudgetReport,
    pub focus_terms: Vec<String>,
    pub max_output_tokens: usize,
    pub queue: Option<TaskInspectQueueReport>,
    pub lease: Option<TaskInspectLeaseReport>,
    pub approval: Option<TaskInspectApprovalReport>,
    pub commit: Option<TaskInspectCommitReport>,
    pub workspace: TaskInspectWorkspaceReport,
    pub diff: Option<TaskInspectDiffReport>,
    pub event_summary: TaskInspectEventSummary,
    pub source_of_truth: String,
    pub projection_kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskQueueInspectRequest {
    pub session_id: Option<Uuid>,
    pub now_ms: Option<i64>,
}

impl TaskQueueInspectRequest {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            session_id: None,
            now_ms: None,
        }
    }

    #[must_use]
    pub const fn with_session_id(mut self, session_id: Uuid) -> Self {
        self.session_id = Some(session_id);
        self
    }

    #[must_use]
    pub const fn with_now_ms(mut self, now_ms: i64) -> Self {
        self.now_ms = Some(now_ms);
        self
    }
}

impl Default for TaskQueueInspectRequest {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskQueueInspectStatusCount {
    pub status: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskQueueInspectTaskReport {
    pub session_id: Uuid,
    pub task_id: Uuid,
    pub task_status: Option<String>,
    pub queue_status: String,
    pub status_class: String,
    pub lease_state: String,
    pub worker_id: Option<String>,
    pub lease_id: Option<Uuid>,
    pub lease_deadline_ms: Option<i64>,
    pub retry_count: i32,
    pub max_retries: i32,
    pub stop_reason: Option<String>,
    pub last_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskQueueInspectReport {
    pub session_id: Option<Uuid>,
    pub now_ms: i64,
    pub empty: bool,
    pub total_count: usize,
    pub active_leased_count: usize,
    pub expired_looking_leased_count: usize,
    pub queue_status_counts: Vec<TaskQueueInspectStatusCount>,
    pub status_class_counts: Vec<TaskQueueInspectStatusCount>,
    pub task_status_counts: Vec<TaskQueueInspectStatusCount>,
    pub lease_state_counts: Vec<TaskQueueInspectStatusCount>,
    pub tasks: Vec<TaskQueueInspectTaskReport>,
    pub source_of_truth: String,
    pub projection_kind: String,
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
pub struct LeaseNextTaskRequest {
    pub worker_id: String,
    pub lease_duration_ms: u64,
}

impl LeaseNextTaskRequest {
    #[must_use]
    pub fn new(worker_id: impl Into<String>) -> Self {
        Self {
            worker_id: worker_id.into(),
            lease_duration_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatTaskLeaseRequest {
    pub worker_id: String,
    pub lease_duration_ms: u64,
}

impl HeartbeatTaskLeaseRequest {
    #[must_use]
    pub fn new(worker_id: impl Into<String>) -> Self {
        Self {
            worker_id: worker_id.into(),
            lease_duration_ms: 60_000,
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
pub enum RuntimeModelProvider {
    DeterministicFake,
    OpenAiCompatible {
        model_id: String,
        api_key_env: String,
    },
}

impl RuntimeModelProvider {
    #[must_use]
    pub fn deterministic_fake() -> Self {
        Self::DeterministicFake
    }

    #[must_use]
    pub fn openai_compatible(model_id: impl Into<String>) -> Self {
        Self::OpenAiCompatible {
            model_id: model_id.into(),
            api_key_env: DEFAULT_OPENAI_COMPATIBLE_API_KEY_ENV.to_owned(),
        }
    }

    #[must_use]
    pub fn provider_kind(&self) -> &'static str {
        match self {
            Self::DeterministicFake => DETERMINISTIC_FAKE_PROVIDER_KIND,
            Self::OpenAiCompatible { .. } => OPENAI_COMPATIBLE_PROVIDER_KIND,
        }
    }

    #[must_use]
    pub fn model_id(&self) -> &str {
        match self {
            Self::DeterministicFake => DETERMINISTIC_FAKE_MODEL_ID,
            Self::OpenAiCompatible { model_id, .. } => model_id,
        }
    }

    #[must_use]
    pub fn provider_name(&self) -> &str {
        self.model_id()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProviderRoutingRequest {
    pub provider: RuntimeModelProvider,
    pub task: String,
    pub context: SessionContextCompileRequest,
    pub max_output_tokens: usize,
}

impl ModelProviderRoutingRequest {
    #[must_use]
    pub fn deterministic_fake(task: impl Into<String>) -> Self {
        Self {
            provider: RuntimeModelProvider::DeterministicFake,
            task: task.into(),
            context: SessionContextCompileRequest::default(),
            max_output_tokens: 256,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelProviderRoutingStatus {
    Responded,
    Skipped,
    Error,
}

impl ModelProviderRoutingStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Responded => "responded",
            Self::Skipped => "skipped",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProviderRoutingResult {
    pub session_id: Uuid,
    pub provider: String,
    pub provider_kind: String,
    pub model_id: String,
    pub request_id: String,
    pub status: ModelProviderRoutingStatus,
    pub summary: Option<String>,
    pub skipped_reason: Option<String>,
    pub error_kind: Option<String>,
    pub safe_error_message: Option<String>,
    pub patch: Option<FilePatchProposal>,
    pub prompt_tokens: Option<usize>,
    pub completion_tokens: Option<usize>,
    pub max_output_tokens: usize,
    pub usage_known: bool,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeModelTurnResult {
    pub session_id: Uuid,
    pub provider: String,
    pub provider_kind: String,
    pub model_id: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalCommitInspectRequest {
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
}

impl ApprovalCommitInspectRequest {
    #[must_use]
    pub const fn new(session_id: Uuid) -> Self {
        Self {
            session_id,
            task_id: None,
        }
    }

    #[must_use]
    pub const fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalCommitDiffEvidence {
    pub repo_path: Option<String>,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalCommitWorkspaceEvidence {
    pub worker_workspace_path: Option<String>,
    pub worker_worktree_path: Option<String>,
    pub allocated_worktree_path: Option<String>,
    pub session_repo_path: Option<String>,
    pub diff_repo_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingApprovalInspectEntry {
    pub task_id: Option<Uuid>,
    pub summary: String,
    pub diff: Option<ApprovalCommitDiffEvidence>,
    pub workspace: ApprovalCommitWorkspaceEvidence,
    pub event_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalDecisionInspectEntry {
    pub task_id: Option<Uuid>,
    pub state: String,
    pub actor: Option<String>,
    pub reason: String,
    pub rejection_reason: Option<String>,
    pub event_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitHandoffInspectEntry {
    pub task_id: Option<Uuid>,
    pub state: String,
    pub actor: Option<String>,
    pub repo_path: String,
    pub message: String,
    pub commit_sha: Option<String>,
    pub failure_reason: Option<String>,
    pub event_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalCommitInspectReport {
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub scope: String,
    pub pending_approvals: Vec<PendingApprovalInspectEntry>,
    pub decisions: Vec<ApprovalDecisionInspectEntry>,
    pub commits: Vec<CommitHandoffInspectEntry>,
    pub pending_count: usize,
    pub decision_count: usize,
    pub commit_count: usize,
    pub event_count: usize,
    pub source_of_truth: String,
    pub projection_kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerLaneDiffInspectRequest {
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub payload_limit_bytes: usize,
}

impl WorkerLaneDiffInspectRequest {
    #[must_use]
    pub const fn new(session_id: Uuid) -> Self {
        Self {
            session_id,
            task_id: None,
            payload_limit_bytes: DEFAULT_TIMELINE_PAYLOAD_LIMIT_BYTES,
        }
    }

    #[must_use]
    pub const fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    #[must_use]
    pub const fn with_payload_limit_bytes(mut self, payload_limit_bytes: usize) -> Self {
        self.payload_limit_bytes = payload_limit_bytes;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerLaneBudgetInspect {
    pub max_prompt_tokens: usize,
    pub max_output_tokens: usize,
    pub max_stdout_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerLaneRequestInspectEntry {
    pub task: BoundedPayloadSummary,
    pub workspace_path: String,
    pub worktree_path: Option<String>,
    pub timeout_ms: u64,
    pub cancellation_requested: bool,
    pub budget: WorkerLaneBudgetInspect,
    pub event_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerLanePolicyInspectEntry {
    pub tool_name: String,
    pub decision: String,
    pub reason: String,
    pub event_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerLaneStateInspectEntry {
    pub from_state: Option<String>,
    pub to_state: String,
    pub reason: String,
    pub event_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerLaneWorkspaceInspectEntry {
    pub requested_workspace_path: Option<String>,
    pub requested_worktree_path: Option<String>,
    pub allocated_worktree_path: Option<String>,
    pub session_repo_path: Option<String>,
    pub task_repo_path: Option<String>,
    pub diff_repo_path: Option<String>,
    pub base_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerLaneObservationInspectEntry {
    pub status: String,
    pub exit_code: Option<i32>,
    pub stdout: BoundedPayloadSummary,
    pub stderr: BoundedPayloadSummary,
    pub duration_ms: u64,
    pub prompt_tokens: Option<usize>,
    pub completion_tokens: Option<usize>,
    pub usage_confidence: String,
    pub skipped_or_unavailable_reason: Option<String>,
    pub event_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerLaneInspectEntry {
    pub task_id: Option<Uuid>,
    pub lane_id: String,
    pub lane_kind: String,
    pub request_status: Option<String>,
    pub request: Option<WorkerLaneRequestInspectEntry>,
    pub policy: Option<WorkerLanePolicyInspectEntry>,
    pub states: Vec<WorkerLaneStateInspectEntry>,
    pub current_state: Option<String>,
    pub is_running: bool,
    pub is_terminal: bool,
    pub terminal_state: Option<String>,
    pub terminal_reason: Option<String>,
    pub failure_reason: Option<String>,
    pub timeout_reason: Option<String>,
    pub cancellation_reason: Option<String>,
    pub workspace: WorkerLaneWorkspaceInspectEntry,
    pub observation: Option<WorkerLaneObservationInspectEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerLaneDiffEvidenceInspectEntry {
    pub task_id: Option<Uuid>,
    pub repo_path: Option<String>,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub paths: Vec<String>,
    pub git_status: BoundedPayloadSummary,
    pub git_diff: BoundedPayloadSummary,
    pub event_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerLaneDiffInspectReport {
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub scope: String,
    pub lane_count: usize,
    pub diff_count: usize,
    pub event_count: usize,
    pub payload_limit_bytes: usize,
    pub lanes: Vec<WorkerLaneInspectEntry>,
    pub diffs: Vec<WorkerLaneDiffEvidenceInspectEntry>,
    pub source_of_truth: String,
    pub projection_kind: String,
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
pub struct TaskQueueProjection {
    pub status: String,
    pub reason: Option<String>,
    pub retry_count: Option<i32>,
    pub max_retries: Option<i32>,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskLeaseProjection {
    pub lease_id: Uuid,
    pub worker_id: String,
    pub status: String,
    pub lease_deadline_ms: Option<i64>,
    pub reason: Option<String>,
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
    pub queue: Option<TaskQueueProjection>,
    pub lease: Option<TaskLeaseProjection>,
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
pub struct LeasedCodexWorkerTaskRequest {
    pub worker_id: String,
    pub lease_duration_ms: u64,
    pub worker: CodexWorkerLaneRequest,
}

impl LeasedCodexWorkerTaskRequest {
    #[must_use]
    pub fn new(worker_id: impl Into<String>, worker: CodexWorkerLaneRequest) -> Self {
        Self {
            worker_id: worker_id.into(),
            lease_duration_ms: 60_000,
            worker,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeasedCodexWorkerTaskResult {
    pub task: TaskProjection,
    pub worker: CodexWorkerLaneResult,
    pub lease_id: Uuid,
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

    #[must_use]
    pub fn architecture_report() -> RuntimeArchitectureReport {
        runtime_architecture_report()
    }

    #[must_use]
    pub fn data_flow_report() -> RuntimeDataFlowReport {
        runtime_data_flow_report()
    }

    #[must_use]
    pub fn storage_report() -> RuntimeStorageReport {
        runtime_storage_report()
    }

    #[must_use]
    pub fn capability_registry() -> CapabilityRegistryReport {
        runtime_capability_registry()
    }

    pub fn capability(capability_id: &str) -> HarnessResult<CapabilityNode> {
        runtime_capability_registry()
            .capabilities
            .into_iter()
            .find(|capability| capability.id == capability_id)
            .ok_or_else(|| HarnessError::new(format!("capability not found: {capability_id}")))
    }

    pub fn capability_gate(capability_id: &str) -> HarnessResult<CapabilityGateReport> {
        let registry = runtime_capability_registry();
        let capability = registry
            .capabilities
            .iter()
            .find(|capability| capability.id == capability_id)
            .cloned()
            .ok_or_else(|| HarnessError::new(format!("capability not found: {capability_id}")))?;
        Ok(capability_gate_report(capability, &registry.capabilities))
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

    pub fn inspect_session(&self, session_id: Uuid) -> HarnessResult<SessionInspectReport> {
        let events = self.event_store.events_for_session(session_id)?;
        session_inspect_report_from_events(session_id, &events)
    }

    pub fn inspect_event_timeline(
        &self,
        request: EventTimelineInspectRequest,
    ) -> HarnessResult<EventTimelineReport> {
        let events = self.event_store.events_for_session(request.session_id)?;
        event_timeline_report_from_events(request, &events)
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

    pub fn inspect_task(
        &self,
        session_id: Uuid,
        task_id: Uuid,
    ) -> HarnessResult<TaskInspectReport> {
        let events = self.event_store.events_for_session(session_id)?;
        project_session(session_id, &events)?;
        let tasks = task_projections_from_events(session_id, &events)?;
        let task = tasks
            .into_iter()
            .find(|task| task.task_id == task_id)
            .ok_or_else(|| HarnessError::new(format!("task not found: {task_id}")))?;
        let queue = self.event_store.task_queue_record(task_id)?;

        task_inspect_report_from_projection(task, queue.as_ref(), &events)
    }

    pub fn inspect_task_queue(
        &self,
        request: TaskQueueInspectRequest,
    ) -> HarnessResult<TaskQueueInspectReport> {
        let now_ms = request.now_ms.map_or_else(current_time_ms, Ok)?;
        if let Some(session_id) = request.session_id {
            let events = self.event_store.events_for_session(session_id)?;
            project_session(session_id, &events)?;
        }

        let records = self.event_store.task_queue_records(request.session_id)?;
        let mut session_ids = records
            .iter()
            .map(|record| record.session_id)
            .collect::<Vec<_>>();
        session_ids.sort_unstable();
        session_ids.dedup();

        let mut task_statuses = BTreeMap::new();
        for session_id in session_ids {
            let events = self.event_store.events_for_session(session_id)?;
            for task in task_projections_from_events(session_id, &events)? {
                task_statuses.insert(task.task_id, task.status);
            }
        }

        Ok(task_queue_inspect_report_from_records(
            request.session_id,
            now_ms,
            records,
            &task_statuses,
        ))
    }

    pub fn inspect_approval_commit(
        &self,
        request: ApprovalCommitInspectRequest,
    ) -> HarnessResult<ApprovalCommitInspectReport> {
        let events = self.event_store.events_for_session(request.session_id)?;
        project_session(request.session_id, &events)?;
        if let Some(task_id) = request.task_id {
            let tasks = task_projections_from_events(request.session_id, &events)?;
            if !tasks.iter().any(|task| task.task_id == task_id) {
                return Err(HarnessError::new(format!("task not found: {task_id}")));
            }
        }

        approval_commit_inspect_report_from_events(request, &events)
    }

    pub fn inspect_worker_lane_diff(
        &self,
        request: WorkerLaneDiffInspectRequest,
    ) -> HarnessResult<WorkerLaneDiffInspectReport> {
        let events = self.event_store.events_for_session(request.session_id)?;
        project_session(request.session_id, &events)?;
        if let Some(task_id) = request.task_id {
            let tasks = task_projections_from_events(request.session_id, &events)?;
            if !tasks.iter().any(|task| task.task_id == task_id) {
                return Err(HarnessError::new(format!("task not found: {task_id}")));
            }
        }

        worker_lane_diff_inspect_report_from_events(request, &events)
    }

    pub fn enqueue_task(
        &mut self,
        session_id: Uuid,
        task_id: Uuid,
    ) -> HarnessResult<TaskProjection> {
        self.enqueue_task_with_max_retries(session_id, task_id, 1)
    }

    pub fn enqueue_task_with_max_retries(
        &mut self,
        session_id: Uuid,
        task_id: Uuid,
        max_retries: i32,
    ) -> HarnessResult<TaskProjection> {
        if max_retries < 0 {
            return Err(HarnessError::new("max retries cannot be negative"));
        }

        self.show_task(session_id, task_id)?;
        self.event_store
            .enqueue_task(session_id, task_id, max_retries, |record| {
                NewEvent::task_enqueued(
                    session_id,
                    TaskQueuePayload::new(
                        task_id,
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

        self.show_task(session_id, task_id)
    }

    pub fn lease_next_task(
        &mut self,
        request: LeaseNextTaskRequest,
    ) -> HarnessResult<Option<TaskProjection>> {
        self.lease_next_task_matching(request, None)
    }

    fn lease_next_task_for_worker_lane(
        &mut self,
        request: LeaseNextTaskRequest,
        worker_lane_kind: &str,
    ) -> HarnessResult<Option<TaskProjection>> {
        if worker_lane_kind.trim().is_empty() {
            return Err(HarnessError::new("worker lane kind cannot be empty"));
        }

        self.lease_next_task_matching(request, Some(worker_lane_kind.trim()))
    }

    fn lease_next_task_matching(
        &mut self,
        request: LeaseNextTaskRequest,
        worker_lane_kind: Option<&str>,
    ) -> HarnessResult<Option<TaskProjection>> {
        if request.worker_id.trim().is_empty() {
            return Err(HarnessError::new("worker id cannot be empty"));
        }

        if request.lease_duration_ms == 0 {
            return Err(HarnessError::new("lease duration must be positive"));
        }

        let lease_id = Uuid::new_v4();
        let lease_deadline_ms = current_time_ms()?.saturating_add(
            i64::try_from(request.lease_duration_ms)
                .map_err(|_| HarnessError::new("lease duration is out of range"))?,
        );
        let record = if let Some(worker_lane_kind) = worker_lane_kind {
            self.event_store.lease_next_queued_task_for_worker_lane(
                request.worker_id.trim(),
                lease_id,
                lease_deadline_ms,
                worker_lane_kind,
                |record| {
                    NewEvent::task_lease_acquired(
                        record.session_id,
                        task_lease_payload_from_record(record)?,
                    )
                },
            )?
        } else {
            self.event_store.lease_next_queued_task(
                request.worker_id.trim(),
                lease_id,
                lease_deadline_ms,
                |record| {
                    NewEvent::task_lease_acquired(
                        record.session_id,
                        task_lease_payload_from_record(record)?,
                    )
                },
            )?
        };
        let Some(record) = record else {
            return Ok(None);
        };

        self.show_task(record.session_id, record.task_id).map(Some)
    }

    pub fn heartbeat_task_lease(
        &mut self,
        task_id: Uuid,
        lease_id: Uuid,
        request: HeartbeatTaskLeaseRequest,
    ) -> HarnessResult<TaskProjection> {
        if request.worker_id.trim().is_empty() {
            return Err(HarnessError::new("worker id cannot be empty"));
        }

        if request.lease_duration_ms == 0 {
            return Err(HarnessError::new("lease duration must be positive"));
        }

        let now_ms = current_time_ms()?;
        let lease_deadline_ms = now_ms.saturating_add(
            i64::try_from(request.lease_duration_ms)
                .map_err(|_| HarnessError::new("lease duration is out of range"))?,
        );
        let record = self.event_store.renew_task_lease(
            task_id,
            lease_id,
            request.worker_id.trim(),
            now_ms,
            lease_deadline_ms,
            |record| {
                NewEvent::task_lease_renewed(
                    record.session_id,
                    task_lease_payload_from_record(record)?,
                )
            },
        )?;

        self.show_task(record.session_id, task_id)
    }

    pub fn expire_task_leases(&mut self, now_ms: i64) -> HarnessResult<Vec<TaskProjection>> {
        let expired = self.event_store.expire_due_task_leases(now_ms, |record| {
            let queue_event = if record.queue.status == TASK_RETRY_QUEUED_STATE {
                NewEvent::task_retry_queued(
                    record.queue.session_id,
                    task_queue_payload_from_record(&record.queue)?,
                )?
            } else {
                NewEvent::task_retry_stopped(
                    record.queue.session_id,
                    task_queue_payload_from_record(&record.queue)?,
                )?
            };

            Ok(vec![
                NewEvent::task_lease_expired(
                    record.queue.session_id,
                    expired_lease_payload_from_record(record)?,
                )?,
                queue_event,
            ])
        })?;
        let mut projections = Vec::new();

        for record in expired {
            projections.push(self.show_task(record.queue.session_id, record.queue.task_id)?);
        }

        Ok(projections)
    }

    pub fn complete_task_lease(
        &mut self,
        task_id: Uuid,
        lease_id: Uuid,
        reason: impl Into<String>,
    ) -> HarnessResult<TaskProjection> {
        self.finish_task_lease(task_id, lease_id, TASK_COMPLETED_STATE, reason)
    }

    pub fn fail_task_lease(
        &mut self,
        task_id: Uuid,
        lease_id: Uuid,
        reason: impl Into<String>,
    ) -> HarnessResult<TaskProjection> {
        self.finish_task_lease(task_id, lease_id, TASK_FAILED_STATE, reason)
    }

    pub fn cancel_task_lease(
        &mut self,
        task_id: Uuid,
        lease_id: Uuid,
        reason: impl Into<String>,
    ) -> HarnessResult<TaskProjection> {
        self.finish_task_lease(task_id, lease_id, TASK_CANCELLED_STATE, reason)
    }

    fn finish_task_lease(
        &mut self,
        task_id: Uuid,
        lease_id: Uuid,
        status: &str,
        reason: impl Into<String>,
    ) -> HarnessResult<TaskProjection> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(HarnessError::new(
                "task lease transition reason is required",
            ));
        }

        let record = self.event_store.transition_leased_task(
            task_id,
            lease_id,
            status,
            reason.trim(),
            current_time_ms()?,
            |record| {
                let payload = task_lease_payload_from_record(record)?;
                match status {
                    TASK_COMPLETED_STATE => {
                        NewEvent::task_lease_completed(record.session_id, payload)
                    }
                    TASK_FAILED_STATE => NewEvent::task_lease_failed(record.session_id, payload),
                    TASK_CANCELLED_STATE => {
                        NewEvent::task_lease_cancelled(record.session_id, payload)
                    }
                    other => Err(HarnessError::new(format!(
                        "unsupported task status: {other}"
                    ))),
                }
            },
        )?;

        self.show_task(record.session_id, task_id)
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

    pub fn approve_task_pending_diff(
        &mut self,
        session_id: Uuid,
        task_id: Uuid,
    ) -> HarnessResult<TaskProjection> {
        let task = self.show_task(session_id, task_id)?;
        let approval = task
            .approval
            .as_ref()
            .ok_or_else(|| HarnessError::new("task pending approval was not recorded"))?;
        if approval.state != PENDING_COMMIT_APPROVAL_STATE {
            return Err(HarnessError::new(format!(
                "task pending diff is already {}",
                approval.state
            )));
        }

        self.event_store.append_event(NewEvent::commit_approved(
            session_id,
            CommitApprovalDecisionPayload::new(
                COMMIT_APPROVED_STATE,
                "approved by runtime approval state machine",
                "runtime",
            )
            .with_task_id(task_id),
        )?)?;

        self.show_task(session_id, task_id)
    }

    pub fn reject_task_pending_diff(
        &mut self,
        session_id: Uuid,
        task_id: Uuid,
        reason: impl Into<String>,
    ) -> HarnessResult<TaskProjection> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(HarnessError::new("rejection reason is required"));
        }

        let task = self.show_task(session_id, task_id)?;
        let approval = task
            .approval
            .as_ref()
            .ok_or_else(|| HarnessError::new("task pending approval was not recorded"))?;
        if approval.state != PENDING_COMMIT_APPROVAL_STATE {
            return Err(HarnessError::new(format!(
                "task pending diff is already {}",
                approval.state
            )));
        }

        self.event_store.append_event(NewEvent::commit_rejected(
            session_id,
            CommitApprovalDecisionPayload::new(COMMIT_REJECTED_STATE, reason.trim(), "runtime")
                .with_task_id(task_id),
        )?)?;

        self.show_task(session_id, task_id)
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

    pub fn commit_approved_task_diff(
        &mut self,
        session_id: Uuid,
        task_id: Uuid,
        message: impl Into<String>,
    ) -> HarnessResult<TaskProjection> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(HarnessError::new("commit message is required"));
        }

        let task = self.show_task(session_id, task_id)?;
        let approval = task
            .approval
            .as_ref()
            .ok_or_else(|| HarnessError::new("task approval was not recorded"))?;
        if approval.state != COMMIT_APPROVED_STATE {
            return Err(HarnessError::new(format!(
                "approved task diff is required before commit; current state is {}",
                approval.state
            )));
        }

        let events = self.event_store.events_for_session(session_id)?;
        if let Some(state) =
            existing_commit_handoff_state_from_events_scoped(&events, Some(task_id))?
        {
            return Err(HarnessError::new(format!(
                "task commit handoff is already {state}"
            )));
        }

        let repo_path = commit_repo_path_from_events_scoped(&events, Some(task_id))?;
        self.event_store.append_event(NewEvent::commit_started(
            session_id,
            CommitHandoffPayload::started(&repo_path, message.trim(), "runtime")
                .with_task_id(task_id),
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
                    )
                    .with_task_id(task_id),
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
                    )
                    .with_task_id(task_id),
                )?)?;
            }
        }

        self.show_task(session_id, task_id)
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

    pub fn run_model_provider_route(
        &mut self,
        session_id: Uuid,
        request: ModelProviderRoutingRequest,
    ) -> HarnessResult<ModelProviderRoutingResult> {
        let context = self.compile_session_context(session_id, request.context)?;
        self.record_model_provider_route(
            session_id,
            request.provider,
            request.task,
            &context,
            request.max_output_tokens,
        )
    }

    fn record_model_provider_route(
        &mut self,
        session_id: Uuid,
        provider: RuntimeModelProvider,
        task: String,
        context: &SessionContextCompileResult,
        max_output_tokens: usize,
    ) -> HarnessResult<ModelProviderRoutingResult> {
        let provider_name = provider.provider_name().to_owned();
        let provider_kind = provider.provider_kind().to_owned();
        let model_id = provider.model_id().to_owned();
        let request_id = Uuid::new_v4().to_string();
        let skipped_configuration_key_name = match &provider {
            RuntimeModelProvider::OpenAiCompatible { api_key_env, .. } => Some(api_key_env.clone()),
            RuntimeModelProvider::DeterministicFake => None,
        };
        let model_request = ModelProviderRequest::new(
            task,
            context.bundle.sources.len(),
            context.bundle.used_bytes,
            max_output_tokens,
        )?;
        self.event_store
            .append_event(NewEvent::model_request_recorded(
                session_id,
                ModelRequestPayload::new(
                    request_id.clone(),
                    provider_name.clone(),
                    provider_kind.clone(),
                    model_id.clone(),
                    model_task_summary(&model_request.task),
                    model_request.context_source_count,
                    model_request.context_used_bytes,
                    model_request.max_output_tokens,
                ),
            )?)?;

        match ModelProviderRouter.route(model_provider_selection(&provider), model_request) {
            Ok(ModelProviderOutcome::Response(response)) => {
                let patch_payload = model_patch_evidence_payload(&response.patch);
                self.event_store
                    .append_event(NewEvent::model_decision_recorded(
                        session_id,
                        ModelDecisionPayload::new(
                            request_id.clone(),
                            provider_name.clone(),
                            response.provider_kind.clone(),
                            response.model_id.clone(),
                            response.summary.clone(),
                            response.usage.prompt_tokens,
                            response.usage.completion_tokens,
                            response.usage.max_output_tokens,
                            true,
                            patch_payload,
                        ),
                    )?)?;

                Ok(ModelProviderRoutingResult {
                    session_id,
                    provider: provider_name,
                    provider_kind: response.provider_kind,
                    model_id: response.model_id,
                    request_id,
                    status: ModelProviderRoutingStatus::Responded,
                    summary: Some(response.summary),
                    skipped_reason: None,
                    error_kind: None,
                    safe_error_message: None,
                    patch: Some(response.patch),
                    prompt_tokens: Some(response.usage.prompt_tokens),
                    completion_tokens: Some(response.usage.completion_tokens),
                    max_output_tokens: response.usage.max_output_tokens,
                    usage_known: true,
                    event_count: self.event_store.events_for_session(session_id)?.len(),
                })
            }
            Ok(ModelProviderOutcome::Skipped(skipped)) => {
                self.event_store
                    .append_event(NewEvent::model_provider_skipped(
                        session_id,
                        ModelProviderSkippedPayload::new(
                            request_id.clone(),
                            provider_name.clone(),
                            skipped.provider_kind.clone(),
                            skipped.model_id.clone(),
                            skipped.reason.clone(),
                            skipped_configuration_key_name,
                        ),
                    )?)?;

                Ok(ModelProviderRoutingResult {
                    session_id,
                    provider: provider_name,
                    provider_kind: skipped.provider_kind,
                    model_id: skipped.model_id,
                    request_id,
                    status: ModelProviderRoutingStatus::Skipped,
                    summary: None,
                    skipped_reason: Some(skipped.reason),
                    error_kind: None,
                    safe_error_message: None,
                    patch: None,
                    prompt_tokens: None,
                    completion_tokens: None,
                    max_output_tokens,
                    usage_known: false,
                    event_count: self.event_store.events_for_session(session_id)?.len(),
                })
            }
            Err(error) => {
                let safe_error_message = sanitize_provider_error_message(error.message());
                self.event_store
                    .append_event(NewEvent::model_provider_error(
                        session_id,
                        ModelProviderErrorPayload::new(
                            request_id.clone(),
                            provider_name,
                            provider_kind,
                            model_id,
                            "provider_error",
                            safe_error_message.clone(),
                            false,
                        ),
                    )?)?;

                Ok(ModelProviderRoutingResult {
                    session_id,
                    provider: provider.provider_name().to_owned(),
                    provider_kind: provider.provider_kind().to_owned(),
                    model_id: provider.model_id().to_owned(),
                    request_id,
                    status: ModelProviderRoutingStatus::Error,
                    summary: None,
                    skipped_reason: None,
                    error_kind: Some("provider_error".to_owned()),
                    safe_error_message: Some(safe_error_message),
                    patch: None,
                    prompt_tokens: None,
                    completion_tokens: None,
                    max_output_tokens,
                    usage_known: false,
                    event_count: self.event_store.events_for_session(session_id)?.len(),
                })
            }
        }
    }

    pub fn run_fake_model_turn(
        &mut self,
        session_id: Uuid,
        request: FakeModelTurnRequest,
    ) -> HarnessResult<FakeModelTurnResult> {
        let context = self.compile_session_context(session_id, request.context)?;
        let routed = self.record_model_provider_route(
            session_id,
            RuntimeModelProvider::DeterministicFake,
            request.task,
            &context,
            request.max_output_tokens,
        )?;
        let Some(patch) = routed.patch.clone() else {
            return Err(HarnessError::new(
                "fake model provider did not return a patch",
            ));
        };
        let prompt_tokens = routed.prompt_tokens.ok_or_else(|| {
            HarnessError::new("fake model provider did not return prompt token usage")
        })?;
        let completion_tokens = routed.completion_tokens.ok_or_else(|| {
            HarnessError::new("fake model provider did not return completion token usage")
        })?;
        let patch_payload = file_patch_payload(&patch);

        let tool_name = harness_tools::APPLY_FILE_PATCH_TOOL.name;
        let intent = FilePatchIntent::new(
            context.bundle.repo_path.clone(),
            patch.path.clone(),
            patch.expected_content.clone(),
            patch.replacement_content.clone(),
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

        let evaluation = evaluate_file_patch(&patch.path, patch.replacement_content.len());

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
            provider: routed.provider,
            provider_kind: routed.provider_kind,
            model_id: routed.model_id,
            patch,
            decision: evaluation.decision,
            reason: evaluation.reason,
            observation,
            prompt_tokens,
            completion_tokens,
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
        self.run_codex_worker_lane_scoped(session_id, None, None, request)
    }

    fn run_codex_worker_lane_scoped(
        &mut self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        repo_path_override: Option<&str>,
        request: CodexWorkerLaneRequest,
    ) -> HarnessResult<CodexWorkerLaneResult> {
        let session = self.show_session(session_id)?;
        let repo_path = repo_path_override.unwrap_or(&session.repo_path).to_owned();

        let lane_id = Uuid::new_v4().to_string();
        let lane_kind = "codex_cli";
        let tool_name = harness_tools::CODEX_WORKER_LANE_TOOL.name;
        let workspace = requested_codex_worker_workspace(&request.workspace, &repo_path);
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
                worker_lane_request_payload(&lane_id, lane_kind, &intent, task_id),
            )?)?;

        let evaluation = evaluate_codex_worker_lane(&CodexWorkerLanePolicyInput {
            task: &intent.task,
            session_repo_path: &repo_path,
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
                task_id,
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
            let allocation = match allocate_task_worktree(&repo_path, &lane_id) {
                Ok(allocation) => allocation,
                Err(error) => {
                    self.append_worker_lane_state(
                        session_id,
                        task_id,
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
                        &repo_path,
                        &allocation.worktree_path,
                        &allocation.base_ref,
                    )
                    .with_optional_task_id(task_id),
                )?)?;

            intent.workspace_path = allocation.worktree_path.clone();
            intent.worktree_path = Some(allocation.worktree_path);
        }

        self.append_worker_lane_state(
            session_id,
            task_id,
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
                task_id,
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
                worker_lane_observation_payload(&lane_id, lane_kind, &observation, task_id),
            )?)?;

        self.append_worker_lane_state(
            session_id,
            task_id,
            &lane_id,
            Some(previous_state),
            observation.status,
            &format!("{} worker lane completed", request.runner.kind()),
        )?;

        let pending_commit_state = if observation.status == WorkerLaneStatus::Succeeded {
            self.record_worker_lane_diff_if_needed(
                session_id,
                task_id,
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

    pub fn run_next_leased_codex_worker_task(
        &mut self,
        request: LeasedCodexWorkerTaskRequest,
    ) -> HarnessResult<Option<LeasedCodexWorkerTaskResult>> {
        if request.lease_duration_ms == 0 {
            return Err(HarnessError::new("lease duration must be positive"));
        }

        if request.worker.timeout_ms == 0 {
            return Err(HarnessError::new(
                "queued codex worker timeout must be positive",
            ));
        }

        if request.worker.budget.max_stdout_bytes == 0 {
            return Err(HarnessError::new(
                "queued codex worker stdout budget must be positive",
            ));
        }

        let lease_duration_ms = effective_queued_worker_lease_duration_ms(
            request.lease_duration_ms,
            request.worker.timeout_ms,
        );
        let Some(leased_task) = self.lease_next_task_for_worker_lane(
            LeaseNextTaskRequest {
                worker_id: request.worker_id.clone(),
                lease_duration_ms,
            },
            DEFAULT_TASK_WORKER_LANE_KIND,
        )?
        else {
            return Ok(None);
        };
        let lease_id = leased_task
            .lease
            .as_ref()
            .ok_or_else(|| HarnessError::new("leased task projection did not include lease"))?
            .lease_id;
        let mut worker_request = request.worker;
        worker_request.task = leased_task.input.clone();
        worker_request.budget.max_prompt_tokens = leased_task.context_budget.max_bytes;
        worker_request.budget.max_output_tokens = leased_task.max_output_tokens;

        let worker = self.run_codex_worker_lane_scoped(
            leased_task.session_id,
            Some(leased_task.task_id),
            Some(&leased_task.repo_path),
            worker_request,
        )?;
        let task = match worker.final_status {
            WorkerLaneStatus::Succeeded => {
                self.complete_task_lease(leased_task.task_id, lease_id, "worker lane succeeded")?
            }
            WorkerLaneStatus::Cancelled => {
                self.cancel_task_lease(leased_task.task_id, lease_id, "worker lane cancelled")?
            }
            WorkerLaneStatus::Failed | WorkerLaneStatus::TimedOut | WorkerLaneStatus::Rejected => {
                self.fail_task_lease(
                    leased_task.task_id,
                    lease_id,
                    format!("worker lane {}", worker.final_status.as_str()),
                )?
            }
            WorkerLaneStatus::Queued | WorkerLaneStatus::Running => {
                return Err(HarnessError::new("worker lane ended in non-terminal state"));
            }
        };

        Ok(Some(LeasedCodexWorkerTaskResult {
            task,
            worker,
            lease_id,
        }))
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
        task_id: Option<Uuid>,
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
                )
                .with_optional_task_id(task_id),
            )?)?;

        Ok(())
    }

    fn record_worker_lane_diff_if_needed(
        &mut self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        repo_path: &str,
        max_diff_bytes: usize,
    ) -> HarnessResult<Option<String>> {
        let Some(evidence) = capture_git_diff_evidence(repo_path, max_diff_bytes)? else {
            return Ok(None);
        };

        self.event_store.append_event(NewEvent::diff_recorded(
            session_id,
            diff_summary_payload(&evidence.summary)
                .with_repo_path(repo_path)
                .with_git_evidence(evidence.git_status, evidence.git_diff)
                .with_optional_task_id(task_id),
        )?)?;
        self.event_store
            .append_event(NewEvent::commit_approval_pending(
                session_id,
                CommitApprovalPendingPayload::new(
                    PENDING_COMMIT_APPROVAL_STATE,
                    "worker lane succeeded with reviewable diff; awaiting human commit approval",
                )
                .with_optional_task_id(task_id),
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

fn runtime_capability_registry() -> CapabilityRegistryReport {
    let capabilities = vec![
        capability_node(
            "runtime-governance",
            "Runtime governance source of truth",
            "foundation",
            "operational",
            "running",
            "The owner needs a runtime-owned truth model instead of chat, UI, or external CLI state.",
            "Architecture, data-flow, storage, session, timeline, task, queue, approval, worker, and commit reports expose source_of_truth fields.",
            "Needs to remain invariant as model entry and internal loop are added.",
            &[],
            &[
                "harness-cli report architecture",
                "harness-cli report data-flow",
                "harness-cli report storage",
                "harness-cli session inspect <session_id>",
                "harness-cli session timeline <session_id>",
            ],
            &[
                "reports expose source_of_truth=EventLog or task_queue+EventLog",
                "derived projections do not mutate runtime state",
                "old EventLog replay remains compatible",
            ],
            &[
                "EventLog is the runtime source of truth",
                "clients and workers are evidence-producing surfaces",
                "derived reports are not authoritative state",
            ],
            &[
                "do not make UI, external CLI session, MCP, skill, hook, or subagent state authoritative",
            ],
            "Keep this invariant green while adding every later capability.",
        ),
        capability_node(
            "task-queue-lease",
            "Task Queue and Lease ownership",
            "foundation",
            "operational",
            "running",
            "The owner needs task execution to be schedulable and leased, not a loose chat turn.",
            "CLI can create, enqueue, lease, heartbeat, expire, retry, stop, complete, fail, and inspect queued tasks.",
            "Current lease path exists; later workers must continue to use it instead of hidden ownership.",
            &["runtime-governance"],
            &[
                "harness-cli session task queue-inspect",
                "harness-cli session task lease-next --worker-id <worker>",
                "harness-cli session task expire-leases",
            ],
            &[
                "leased tasks show worker_id, lease_id, lease_state, retry_count, and stop_reason",
                "expired leases produce expired, retry_queued, or stopped evidence",
            ],
            &[
                "Task Lease grants temporary ownership only",
                "lease transitions pair database scheduling state with EventLog evidence",
            ],
            &["do not use in-memory locks or permanent worker ownership"],
            "Use this as the required scheduler for internal agent loop work.",
        ),
        capability_node(
            "approval-commit-handoff",
            "Approval and Commit Handoff",
            "foundation",
            "operational",
            "running",
            "The owner needs diffs to wait for approval before durable git history is created.",
            "CLI can inspect pending approvals, approve or reject task diffs, and commit only approved diffs.",
            "Rich checkpoint and PR UX are not built; current path is CLI and git commit only.",
            &["runtime-governance", "task-queue-lease"],
            &[
                "harness-cli session approval inspect <session_id>",
                "harness-cli session task approve <session_id> <task_id>",
                "harness-cli session task reject <session_id> <task_id> --reason <reason>",
                "harness-cli session task commit <session_id> <task_id> --message <message>",
            ],
            &[
                "pending diffs cannot commit",
                "rejected diffs cannot commit",
                "approved diffs can enter Commit Handoff",
                "commit failure records failure reason",
            ],
            &[
                "workers propose diffs",
                "runtime owns approval state",
                "runtime owns Commit Handoff",
            ],
            &["do not auto-push", "do not let a worker commit directly"],
            "Keep this as the final mutating boundary for Phase A task output.",
        ),
        capability_node(
            "external-codex-lane",
            "Local governed Codex CLI worker lane",
            "foundation",
            "operational",
            "running_or_skipped",
            "The owner needs one real external agent lane that can produce evidence without becoming the runtime owner.",
            "CLI can attempt local Codex acceptance, explicitly skip when unavailable, capture worker output, diff, policy, and pending approval state.",
            "This proves external-lane governance, not the internal Phase A agent loop.",
            &[
                "runtime-governance",
                "task-queue-lease",
                "approval-commit-handoff",
            ],
            &[
                "harness-cli session codex-acceptance <session_id>",
                "harness-cli session task run-next-codex-worker --worker-id <worker> -- <command>",
                "harness-cli session worker inspect <session_id>",
            ],
            &[
                "unavailable Codex reports codex_acceptance_status=skipped",
                "successful worker run records policy, output, diff, and pending_commit_approval",
            ],
            &[
                "Codex CLI is an evidence-producing worker lane",
                "Codex internal state is not source of truth",
                "worker output and diffs enter EventLog through runtime",
            ],
            &["do not treat Codex Cloud or Codex app state as harness state"],
            "Keep as external-lane proof while internal loop is built separately.",
        ),
        capability_node(
            "observability-replay",
            "Inspect, report, and replay observability",
            "foundation",
            "operational",
            "running",
            "The owner needs to inspect what happened instead of trusting a PR summary.",
            "CLI exposes reports and bounded timelines for sessions, tasks, queue state, worker lanes, approvals, commits, and storage.",
            "Needs Phase A model, loop, and tool events added without schema breakage.",
            &["runtime-governance"],
            &[
                "harness-cli report architecture --json",
                "harness-cli session inspect <session_id> --json",
                "harness-cli session timeline <session_id> --json",
                "harness-cli session worker inspect <session_id> --json",
                "harness-cli session approval inspect <session_id> --json",
            ],
            &[
                "JSON reports parse",
                "bounded payloads report original bytes and truncation",
                "inspect commands do not mutate state",
            ],
            &[
                "EventLog replay drives reports",
                "large payloads are bounded before display",
            ],
            &["do not expose secrets or unbounded stdout/stderr in reports"],
            "Use as the owner-visible surface for every future capability.",
        ),
        capability_node(
            "model-entry",
            "Auditable model entry",
            "phase-a",
            "candidate",
            "not_accepted",
            "The owner needs model providers to enter the runtime as evidence producers, not file writers or approvers.",
            "Expected surface: provider request/response, skipped config, and provider error evidence visible through EventLog and inspect/report.",
            "Draft work exists but is not owner-approved and must pass EventLog compatibility plus secret redaction review.",
            &["runtime-governance", "observability-replay"],
            &[
                "future: harness-cli session model-route <session_id>",
                "future: harness-cli session timeline <session_id> --json",
            ],
            &[
                "fake provider records deterministic evidence",
                "missing real-provider config is skipped",
                "provider error records error evidence",
                "no approval or commit is produced",
                "API keys and env values never appear in EventLog, stdout, stderr, inspect, or report",
            ],
            &[
                "model provider can only produce evidence",
                "model provider cannot mutate files",
                "model provider cannot approve or commit",
            ],
            &[
                "do not build agent loop",
                "do not build tools",
                "do not add MCP, skills, hooks, memory, subagents, UI, daemon, or remote workers",
            ],
            "Owner must explicitly approve model-entry implementation before code can be merged.",
        ),
        capability_node(
            "bounded-agent-loop",
            "Bounded internal agent loop",
            "phase-a",
            "missing",
            "not_built",
            "The owner needs the harness to run an internal task loop instead of delegating only to external lanes.",
            "Expected surface: task run report shows loop steps, budget, stop reason, and terminal state.",
            "Not built. It depends on accepted model entry.",
            &["model-entry", "runtime-governance", "observability-replay"],
            &["future: harness-cli session task run-internal <session_id> <task_id>"],
            &[
                "loop has max turn and budget limits",
                "loop records step evidence",
                "loop stops on done, error, budget, denied action, or required approval",
            ],
            &[
                "runtime owns loop state",
                "loop cannot bypass Policy Gate or Approval State Machine",
            ],
            &["do not add subagents or memory in Phase A"],
            "Wait for model-entry acceptance.",
        ),
        capability_node(
            "controlled-coding-tools",
            "Controlled coding tools",
            "phase-a",
            "missing",
            "not_built",
            "The owner needs file, shell, edit, search, and test actions to be tools under runtime governance.",
            "Expected surface: tool intent, policy decision, execution observation, and denied/ask/error states visible in timeline/report.",
            "Policy and Tool Runtime boundaries exist, but the mature internal coding tool set is not complete.",
            &["bounded-agent-loop", "runtime-governance"],
            &["future: harness-cli session tool inspect <session_id>"],
            &[
                "allowed tools execute through Tool Runtime",
                "denied tools do not execute",
                "ask tools wait for approval",
                "tool output is bounded and redacted",
            ],
            &[
                "Policy Gate is the only authorization surface",
                "Tool Runtime is the only execution surface",
            ],
            &["do not let model output directly mutate files"],
            "Wait for bounded-agent-loop design and owner gate.",
        ),
        capability_node(
            "planning-mode",
            "Planning mode",
            "phase-a",
            "missing",
            "not_built",
            "The owner needs a read-only planning path before implementation begins.",
            "Expected surface: plan output and evidence can be inspected, and the plan mode proves no file mutation.",
            "Not built as an enforceable mode.",
            &["model-entry", "observability-replay"],
            &["future: harness-cli session plan <session_id> --task <task>"],
            &[
                "planning can read and inspect context",
                "planning cannot write files",
                "planning output is evidence, not approval",
            ],
            &[
                "planning mode must enforce write blocking",
                "approval remains runtime-owned",
            ],
            &["do not treat a plan as implementation approval"],
            "Wait until model-entry evidence and read-only boundaries are accepted.",
        ),
        capability_node(
            "end-to-end-task-evidence",
            "Phase A end-to-end task evidence",
            "phase-a",
            "missing",
            "not_built",
            "The owner needs one complete internal task path that can be run and inspected without reading PR summaries.",
            "Expected surface: one CLI acceptance path creates a task, runs the internal loop, uses controlled tools, produces a diff, stops at approval, and exposes replay evidence.",
            "Not built. This is the Phase A AVP, and it depends on model entry, loop, tools, and planning boundaries.",
            &[
                "model-entry",
                "bounded-agent-loop",
                "controlled-coding-tools",
                "planning-mode",
                "approval-commit-handoff",
            ],
            &["future: harness-cli session task accept-phase-a <session_id>"],
            &[
                "task evidence covers input, context, model, tool intents, policy decisions, observations, diff, approval state, and replay summary",
                "normal path stops at pending_commit_approval",
                "failure paths show skipped, error, denied, expired, or rejected states",
            ],
            &[
                "EventLog remains source of truth",
                "Task Lease owns execution",
                "Policy Gate owns tool authorization",
                "Approval State Machine owns approval",
                "Commit Handoff owns commits",
            ],
            &[
                "do not include MCP, skills, hooks, memory, subagents, daemon, UI, or remote workers",
            ],
            "Build only after the four preceding Phase A capabilities are accepted.",
        ),
    ];

    CapabilityRegistryReport {
        report_id: "capability_workbench".to_owned(),
        title: "Capability Workbench".to_owned(),
        summary: "Owner-facing registry of stable runtime capability nodes, their current AVP status, gates, and acceptance surfaces.".to_owned(),
        capability_count: capabilities.len(),
        capabilities,
        source_of_truth: "CONTEXT.md + accepted ADRs + runtime capability registry".to_owned(),
        projection_kind: "capability_registry_report".to_owned(),
    }
}

fn capability_gate_report(
    capability: CapabilityNode,
    all_capabilities: &[CapabilityNode],
) -> CapabilityGateReport {
    let (gate_status, ready_for_implementation, ready_for_merge, blocking_items) =
        match capability.status.as_str() {
            "operational" => (
                "implemented_capability",
                false,
                false,
                vec![
                "No new implementation gate is requested for an already operational capability."
                    .to_owned(),
            ],
            ),
            "candidate" => (
                "blocked_waiting_owner_implementation_approval",
                false,
                false,
                vec![
                    "Owner has not explicitly approved implementation for this capability."
                        .to_owned(),
                    "Owner Acceptance Demo Pack must be accepted before implementation.".to_owned(),
                    "EventLog compatibility and secret redaction review must pass.".to_owned(),
                ],
            ),
            "missing" => (
                "blocked_by_dependencies",
                false,
                false,
                dependency_blocking_items(&capability, all_capabilities),
            ),
            "deferred" => (
                "deferred_to_later_phase",
                false,
                false,
                vec!["Capability is outside the current approved phase.".to_owned()],
            ),
            other => (
                "unknown_status",
                false,
                false,
                vec![format!("Unknown capability status: {other}")],
            ),
        };

    CapabilityGateReport {
        capability_id: capability.id,
        capability_name: capability.name,
        phase: capability.phase,
        status: capability.status,
        avp_status: capability.avp_status,
        gate_status: gate_status.to_owned(),
        ready_for_implementation,
        ready_for_merge,
        blocking_items,
        required_evidence: capability.acceptance_gates,
        source_of_truth: "capability registry + owner acceptance gate".to_owned(),
        projection_kind: "capability_gate_report".to_owned(),
    }
}

fn dependency_blocking_items(
    capability: &CapabilityNode,
    all_capabilities: &[CapabilityNode],
) -> Vec<String> {
    let mut blocking_items = Vec::new();

    for dependency in &capability.dependencies {
        match all_capabilities
            .iter()
            .find(|capability| capability.id == *dependency)
        {
            Some(dependency_capability) if dependency_capability.status == "operational" => {}
            Some(dependency_capability) => blocking_items.push(format!(
                "Dependency capability is not accepted: {} ({})",
                dependency_capability.id, dependency_capability.status
            )),
            None => blocking_items.push(format!(
                "Dependency capability is not registered: {dependency}"
            )),
        }
    }

    if blocking_items.is_empty() {
        blocking_items.push(
            "Capability is missing implementation but its registered dependencies are operational."
                .to_owned(),
        );
    }

    blocking_items
}

#[allow(clippy::too_many_arguments)]
fn capability_node(
    id: &str,
    name: &str,
    phase: &str,
    status: &str,
    avp_status: &str,
    problem: &str,
    user_visible_result: &str,
    current_gap: &str,
    dependencies: &[&str],
    operation_entries: &[&str],
    acceptance_gates: &[&str],
    runtime_boundaries: &[&str],
    non_goals: &[&str],
    next_owner_gate: &str,
) -> CapabilityNode {
    CapabilityNode {
        id: id.to_owned(),
        name: name.to_owned(),
        phase: phase.to_owned(),
        status: status.to_owned(),
        avp_status: avp_status.to_owned(),
        problem: problem.to_owned(),
        user_visible_result: user_visible_result.to_owned(),
        current_gap: current_gap.to_owned(),
        dependencies: strings(dependencies),
        operation_entries: strings(operation_entries),
        acceptance_gates: strings(acceptance_gates),
        runtime_boundaries: strings(runtime_boundaries),
        non_goals: strings(non_goals),
        next_owner_gate: next_owner_gate.to_owned(),
    }
}

fn runtime_architecture_report() -> RuntimeArchitectureReport {
    RuntimeArchitectureReport {
        report_id: "runtime_architecture".to_owned(),
        title: "Runtime Architecture Report".to_owned(),
        summary:
            "Deterministic topology for the current v0.1-v0.3 Rust coding-agent harness runtime."
                .to_owned(),
        boundary_adrs: runtime_boundary_adrs(),
        sections: vec![
            report_section(
                "control_surfaces",
                "Control Surfaces",
                "CLI/API requests enter the harness through Runtime-owned commands and report APIs.",
                &["CLI/API is a client surface, not the source of truth."],
            ),
            report_section(
                "governance",
                "Governance",
                "Policy Gate authorizes mutating intents before Tool Runtime or worker lanes can execute.",
                &["Policy Gate authorizes; Tool Runtime executes allowed calls."],
            ),
            report_section(
                "execution",
                "Execution",
                "Tasks run through internal turns or governed worker lanes in Task Worktrees.",
                &["Local Governed Codex CLI Worker is evidence-producing, not authoritative."],
            ),
            report_section(
                "storage_and_replay",
                "Storage And Replay",
                "EventLog records meaningful runtime facts and PostgreSQL task_queue supports scheduling state.",
                &["EventLog remains the source of truth; projections are derived."],
            ),
            report_section(
                "acceptance",
                "Acceptance",
                "Vertical Acceptance Path proves one observable path through CLI, EventLog, policy, worker execution, approval, and replay.",
                &["Acceptance is end-to-end evidence, not a dashboard-only milestone."],
            ),
        ],
        nodes: vec![
            report_node(
                "cli_api",
                "CLI/API",
                "control_surface",
                "Client entry points for sessions, tasks, approvals, commits, and inspect reports.",
                &["Session", "Task"],
                &[],
            ),
            report_node(
                "runtime",
                "Runtime",
                "orchestrator",
                "Harness-owned coordinator for tasks, EventLog appends, policy checks, worker lanes, approvals, and projections.",
                &["Session", "Task", "Vertical Acceptance Path"],
                &["ADR-0001", "ADR-0004"],
            ),
            report_node(
                "policy_gate",
                "Policy Gate",
                "authorization",
                "The only authorization surface for mutating tool and lane intents.",
                &["Lane-Level Governance"],
                &["ADR-0002", "ADR-0007"],
            ),
            report_node(
                "tool_runtime",
                "Tool Runtime",
                "execution_surface",
                "Executes allowed file patch, verification command, and worker lane calls and records observations.",
                &["Lane-Level Governance"],
                &["ADR-0002"],
            ),
            report_node(
                "eventlog",
                "EventLog",
                "source_of_truth",
                "Append-only PostgreSQL-backed event stream used for replay, audit, and derived projections.",
                &["Session", "Task", "Approval State Machine"],
                &["ADR-0001", "ADR-0004"],
            ),
            report_node(
                "postgres_task_queue",
                "PostgreSQL task_queue",
                "runtime_scheduling_state",
                "PostgreSQL-backed queue and lease table for queued, leased, retry, and terminal task scheduling state.",
                &["Task Lease", "Task"],
                &["ADR-0004"],
            ),
            report_node(
                "local_governed_codex_cli_worker",
                "Local Governed Codex CLI Worker",
                "worker_lane",
                "First governed external agent lane; produces captured output, usage evidence, and diffs under Runtime control.",
                &[
                    "Local Governed Codex CLI Worker",
                    "One-Shot Worker Run",
                    "First Real Lane",
                ],
                &["ADR-0007"],
            ),
            report_node(
                "task_worktree",
                "Task Worktree",
                "workspace_boundary",
                "Per-task git worktree used to isolate worker edits from the user's current working tree and other tasks.",
                &["Task Worktree"],
                &["ADR-0007"],
            ),
            report_node(
                "approval_state_machine",
                "Approval State Machine",
                "approval_boundary",
                "Runtime-owned pending approval, approval, rejection, and final approval outcome state.",
                &["Approval State Machine"],
                &["ADR-0001", "ADR-0007"],
            ),
            report_node(
                "commit_handoff",
                "Commit Handoff",
                "git_boundary",
                "Harness-owned step that turns an approved Task Worktree diff into durable git history without auto-push.",
                &["Commit Handoff"],
                &["ADR-0007"],
            ),
            report_node(
                "task_lease",
                "Task Lease",
                "ownership_claim",
                "Temporary PostgreSQL-backed worker ownership claim for one queued task.",
                &["Task Lease"],
                &["ADR-0004"],
            ),
            report_node(
                "vertical_acceptance_path",
                "Vertical Acceptance Path",
                "release_slice",
                "Observable end-to-end path through CLI, PostgreSQL EventLog, policy, worker execution, approval, and replayable evidence.",
                &["Vertical Acceptance Path"],
                &["ADR-0001", "ADR-0002", "ADR-0004", "ADR-0007"],
            ),
        ],
        edges: vec![
            report_edge(
                "cli_to_runtime",
                "cli_api",
                "runtime",
                "invokes",
                "CLI/API calls Runtime-owned commands and inspect reports.",
            ),
            report_edge(
                "runtime_to_eventlog",
                "runtime",
                "eventlog",
                "appends",
                "Runtime appends meaningful facts before deriving projections.",
            ),
            report_edge(
                "runtime_to_queue",
                "runtime",
                "postgres_task_queue",
                "schedules",
                "Runtime mirrors queue and lease state in PostgreSQL task_queue.",
            ),
            report_edge(
                "runtime_to_policy",
                "runtime",
                "policy_gate",
                "requests_authorization",
                "Runtime asks Policy Gate before mutating tool or worker execution.",
            ),
            report_edge(
                "policy_to_tool_runtime",
                "policy_gate",
                "tool_runtime",
                "allows_execution",
                "Tool Runtime executes only allowed policy decisions.",
            ),
            report_edge(
                "runtime_to_worktree",
                "runtime",
                "task_worktree",
                "allocates",
                "Runtime allocates per-task worktrees for governed worker edits.",
            ),
            report_edge(
                "lease_to_worker",
                "task_lease",
                "local_governed_codex_cli_worker",
                "grants_temporary_ownership",
                "A lease gives one worker temporary ownership of a queued task.",
            ),
            report_edge(
                "worker_to_eventlog",
                "local_governed_codex_cli_worker",
                "eventlog",
                "produces_evidence",
                "Worker output, status, usage, and diff evidence are captured through Runtime into EventLog.",
            ),
            report_edge(
                "approval_to_commit",
                "approval_state_machine",
                "commit_handoff",
                "gates",
                "Only approved diffs enter commit handoff.",
            ),
            report_edge(
                "acceptance_covers_runtime",
                "vertical_acceptance_path",
                "runtime",
                "proves",
                "Acceptance validates the observable runtime path end to end.",
            ),
        ],
        non_goals: runtime_report_non_goals(),
        source_of_truth: "CONTEXT.md + accepted ADRs + runtime contract".to_owned(),
        projection_kind: "runtime_architecture_report".to_owned(),
    }
}

fn runtime_data_flow_report() -> RuntimeDataFlowReport {
    let steps = vec![
        data_flow_step(
            "task_creation",
            1,
            "Task creation",
            "runtime",
            &["task.created"],
            "Runtime creates a schedulable Task inside a Session.",
        ),
        data_flow_step(
            "context_compile",
            2,
            "Context compile",
            "runtime",
            &["context.compiled"],
            "Runtime compiles bounded repository context for the task or session.",
        ),
        data_flow_step(
            "enqueue",
            3,
            "Enqueue",
            "postgres_task_queue",
            &["task.enqueued"],
            "Task enters PostgreSQL task_queue as queued scheduling state.",
        ),
        data_flow_step(
            "lease",
            4,
            "Lease acquisition",
            "task_lease",
            &["task.lease_acquired"],
            "One worker receives a temporary Task Lease.",
        ),
        data_flow_step(
            "worker_lane_request",
            5,
            "Worker lane request",
            "local_governed_codex_cli_worker",
            &["worker_lane.requested"],
            "Runtime records the governed worker lane request and budget.",
        ),
        data_flow_step(
            "policy_decision",
            6,
            "Policy decision",
            "policy_gate",
            &["policy.decision"],
            "Policy Gate allows, asks, or denies the worker/tool intent.",
        ),
        data_flow_step(
            "worktree_allocation",
            7,
            "Worktree allocation",
            "task_worktree",
            &["worker_lane.worktree_allocated"],
            "Runtime allocates a Task Worktree for isolated edits.",
        ),
        data_flow_step(
            "worker_observation",
            8,
            "Worker observation",
            "local_governed_codex_cli_worker",
            &["worker_lane.observation_recorded"],
            "Worker output, exit status, duration, and usage evidence are captured.",
        ),
        data_flow_step(
            "diff_recorded",
            9,
            "Diff recorded",
            "runtime",
            &["diff.recorded"],
            "Runtime records changed files, insertions, deletions, and git evidence.",
        ),
        data_flow_step(
            "pending_approval",
            10,
            "Pending approval",
            "approval_state_machine",
            &["commit.approval_pending"],
            "A reviewable diff enters pending commit approval.",
        ),
        data_flow_step(
            "approval_decision",
            11,
            "Approval or rejection",
            "approval_state_machine",
            &["commit.approved", "commit.rejected"],
            "Human or harness approval state moves the task forward or rejects it.",
        ),
        data_flow_step(
            "commit_started",
            12,
            "Commit started",
            "commit_handoff",
            &["commit.started"],
            "Commit Handoff starts only after approval.",
        ),
        data_flow_step(
            "commit_finished",
            13,
            "Commit succeeded or failed",
            "commit_handoff",
            &["commit.succeeded", "commit.failed"],
            "Runtime records the durable commit outcome or failure reason.",
        ),
    ];

    RuntimeDataFlowReport {
        report_id: "runtime_data_flow".to_owned(),
        title: "Runtime Data-Flow Report".to_owned(),
        summary: "Ordered task data flow from creation through context, queue, lease, governed worker execution, approval, and commit handoff.".to_owned(),
        boundary_adrs: runtime_boundary_adrs(),
        edges: data_flow_edges(&steps),
        steps,
        non_goals: runtime_report_non_goals(),
        source_of_truth: "EventLog event contract + PostgreSQL task_queue scheduling state".to_owned(),
        projection_kind: "runtime_data_flow_report".to_owned(),
    }
}

fn runtime_storage_report() -> RuntimeStorageReport {
    RuntimeStorageReport {
        report_id: "runtime_storage".to_owned(),
        title: "Runtime Storage Report".to_owned(),
        summary: "Storage boundaries for EventLog source-of-truth data, PostgreSQL task_queue scheduling state, and rebuildable projections.".to_owned(),
        boundary_adrs: runtime_boundary_adrs(),
        sections: vec![
            report_section(
                "eventlog_boundary",
                "EventLog Boundary",
                "EventLog is the source of truth for sessions, tasks, approvals, policy decisions, observations, recovery, and commit outcomes.",
                &["Runtime facts must be appended before projections or client views rely on them."],
            ),
            report_section(
                "task_queue_boundary",
                "Task Queue Boundary",
                "task_queue is PostgreSQL runtime scheduling state for leases, retry counters, worker ownership, and stop reasons.",
                &["task_queue does not replace EventLog; queue mutations are paired with EventLog evidence."],
            ),
            report_section(
                "projection_boundary",
                "Projection Boundary",
                "Session, Task, approval, commit, timeline, and inspect reports are derived views.",
                &["Derived projections are rebuildable and must not become independent truths."],
            ),
        ],
        tables: vec![
            RuntimeStorageTableReport {
                id: "event_log".to_owned(),
                table_name: "harness_runtime.event_log".to_owned(),
                role: "append_only_source_of_truth".to_owned(),
                ownership_boundary: "Runtime appends meaningful events; clients and workers do not mutate state directly.".to_owned(),
                key_fields: strings(&["event_id", "session_id", "sequence", "event_type", "schema_version", "payload"]),
                derived_from: None,
            },
            RuntimeStorageTableReport {
                id: "task_queue".to_owned(),
                table_name: "harness_runtime.task_queue".to_owned(),
                role: "runtime_scheduling_state".to_owned(),
                ownership_boundary: "Runtime owns queue, lease, retry, and terminal scheduling transitions inside PostgreSQL.".to_owned(),
                key_fields: strings(&["task_id", "session_id", "status", "worker_id", "lease_id", "lease_deadline_ms", "retry_count", "max_retries", "stop_reason"]),
                derived_from: Some("EventLog task events plus transactional queue mutations".to_owned()),
            },
        ],
        projections: vec![
            storage_projection("session_projection", "SessionProjection", "EventLog replay", "Session status, repo path, and event count are derived from session events."),
            storage_projection("task_projection", "TaskProjection", "EventLog replay + task_queue evidence where needed", "Task status, worker output, diff, approval, commit, and lease evidence are derived views."),
            storage_projection("inspect_reports", "Inspect Reports", "EventLog + task_queue read-only queries", "Session, timeline, task, and queue inspect reports expose state without mutating it."),
            storage_projection("approval_commit_projection", "Approval and Commit Projection", "EventLog replay", "Approval State Machine and Commit Handoff state are reconstructed from approval and commit events."),
        ],
        non_goals: runtime_report_non_goals(),
        source_of_truth: "ADR-0001 EventLog discipline inside ADR-0004 PostgreSQL runtime storage".to_owned(),
        projection_kind: "runtime_storage_report".to_owned(),
    }
}

fn runtime_boundary_adrs() -> Vec<String> {
    strings(&["ADR-0001", "ADR-0002", "ADR-0004", "ADR-0007"])
}

fn runtime_report_non_goals() -> Vec<RuntimeReportNonGoal> {
    vec![
        report_non_goal(
            "web_ui",
            "Do not build a visual Web UI or dashboard in this report slice.",
        ),
        report_non_goal(
            "server_sse",
            "Do not add server, SSE, WebSocket, or remote API surfaces.",
        ),
        report_non_goal(
            "distributed_scheduling",
            "Do not add k3s, remote workers, distributed scheduling, or production deployment scope.",
        ),
        report_non_goal(
            "new_lanes",
            "Do not add new worker lanes beyond the current Local Governed Codex CLI Worker contract.",
        ),
        report_non_goal(
            "automatic_push",
            "Do not automatically push user code as part of Commit Handoff.",
        ),
        report_non_goal(
            "replace_eventlog",
            "Do not replace EventLog or make derived projections authoritative.",
        ),
    ]
}

fn data_flow_edges(steps: &[RuntimeDataFlowStep]) -> Vec<RuntimeReportEdge> {
    steps
        .windows(2)
        .map(|pair| {
            report_edge(
                &format!("{}_to_{}", pair[0].id, pair[1].id),
                &pair[0].id,
                &pair[1].id,
                "then",
                "Next ordered step in the task data flow.",
            )
        })
        .collect()
}

fn report_section(
    id: &str,
    title: &str,
    summary: &str,
    key_points: &[&str],
) -> RuntimeReportSection {
    RuntimeReportSection {
        id: id.to_owned(),
        title: title.to_owned(),
        summary: summary.to_owned(),
        key_points: strings(key_points),
    }
}

fn report_node(
    id: &str,
    label: &str,
    kind: &str,
    summary: &str,
    glossary_terms: &[&str],
    adr_refs: &[&str],
) -> RuntimeReportNode {
    RuntimeReportNode {
        id: id.to_owned(),
        label: label.to_owned(),
        kind: kind.to_owned(),
        summary: summary.to_owned(),
        glossary_terms: strings(glossary_terms),
        adr_refs: strings(adr_refs),
    }
}

fn report_edge(id: &str, from: &str, to: &str, label: &str, summary: &str) -> RuntimeReportEdge {
    RuntimeReportEdge {
        id: id.to_owned(),
        from: from.to_owned(),
        to: to.to_owned(),
        label: label.to_owned(),
        summary: summary.to_owned(),
    }
}

fn report_non_goal(id: &str, summary: &str) -> RuntimeReportNonGoal {
    RuntimeReportNonGoal {
        id: id.to_owned(),
        summary: summary.to_owned(),
    }
}

fn data_flow_step(
    id: &str,
    order: usize,
    label: &str,
    component_id: &str,
    event_types: &[&str],
    summary: &str,
) -> RuntimeDataFlowStep {
    RuntimeDataFlowStep {
        id: id.to_owned(),
        order,
        label: label.to_owned(),
        component_id: component_id.to_owned(),
        event_types: strings(event_types),
        summary: summary.to_owned(),
    }
}

fn storage_projection(
    id: &str,
    name: &str,
    source: &str,
    summary: &str,
) -> RuntimeStorageProjectionReport {
    RuntimeStorageProjectionReport {
        id: id.to_owned(),
        name: name.to_owned(),
        source: source.to_owned(),
        summary: summary.to_owned(),
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
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

fn session_inspect_report_from_events(
    session_id: Uuid,
    events: &[EventEnvelope],
) -> HarnessResult<SessionInspectReport> {
    let projection = project_session(session_id, events)?;
    let tasks = task_projections_from_events(session_id, events)?;
    let mut counts = BTreeMap::new();

    for task in &tasks {
        *counts.entry(task.status.clone()).or_insert(0) += 1;
    }

    Ok(SessionInspectReport {
        session_id: projection.session_id,
        repo_path: projection.repo_path,
        status: projection.status.as_str().to_owned(),
        event_count: projection.event_count,
        latest_event_sequence: events.last().map(|event| event.sequence),
        latest_event_type: events
            .last()
            .map(|event| event.event_type.as_str().to_owned()),
        task_count: tasks.len(),
        task_status_counts: counts
            .into_iter()
            .map(|(status, count)| SessionTaskStatusCount { status, count })
            .collect(),
        source_of_truth: "EventLog".to_owned(),
        projection_kind: "derived_from_eventlog".to_owned(),
    })
}

fn event_timeline_report_from_events(
    request: EventTimelineInspectRequest,
    events: &[EventEnvelope],
) -> HarnessResult<EventTimelineReport> {
    project_session(request.session_id, events)?;

    if let Some(task_id) = request.task_id {
        let tasks = task_projections_from_events(request.session_id, events)?;
        if !tasks.iter().any(|task| task.task_id == task_id) {
            return Err(HarnessError::new(format!("task not found: {task_id}")));
        }
    }

    let mut entries = Vec::new();
    for event in events {
        let payload_task_id = payload_task_id(&event.payload)?;
        if request
            .task_id
            .is_some_and(|task_id| payload_task_id != Some(task_id))
        {
            continue;
        }

        entries.push(EventTimelineEntry {
            event_id: event.event_id,
            session_id: event.session_id,
            sequence: event.sequence,
            event_type: event.event_type.as_str().to_owned(),
            schema_version: event.schema_version,
            task_id: payload_task_id,
            payload: bounded_payload_summary(&event.payload, request.payload_limit_bytes)?,
        });
    }

    Ok(EventTimelineReport {
        session_id: request.session_id,
        task_id: request.task_id,
        event_count: entries.len(),
        payload_limit_bytes: request.payload_limit_bytes,
        events: entries,
        source_of_truth: "EventLog".to_owned(),
        projection_kind: "bounded_eventlog_timeline".to_owned(),
    })
}

fn bounded_payload_summary(
    payload: &serde_json::Value,
    limit_bytes: usize,
) -> HarnessResult<BoundedPayloadSummary> {
    let serialized =
        serde_json::to_string(payload).map_err(|error| HarnessError::new(error.to_string()))?;
    Ok(bounded_text_summary(serialized, limit_bytes))
}

fn bounded_text_summary(text: String, limit_bytes: usize) -> BoundedPayloadSummary {
    let original_bytes = text.len();
    if original_bytes <= limit_bytes {
        return BoundedPayloadSummary {
            text,
            original_bytes,
            truncated: false,
        };
    }

    let mut end = limit_bytes;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }

    BoundedPayloadSummary {
        text: text[..end].to_owned(),
        original_bytes,
        truncated: true,
    }
}

fn task_inspect_report_from_projection(
    task: TaskProjection,
    queue_record: Option<&TaskQueueRecord>,
    events: &[EventEnvelope],
) -> HarnessResult<TaskInspectReport> {
    let mut approval_actor = None;
    let mut approval_reason = None;
    let mut commit_actor = None;
    let mut commit_repo_path = None;
    let mut worker_workspace_path = None;
    let mut worker_worktree_path = None;
    let mut allocated_worktree_path = task
        .worktree
        .as_ref()
        .map(|worktree| worktree.worktree_path.clone());
    let mut session_repo_path = None;
    let mut diff_repo_path = None;
    let mut task_event_count = 0;
    let mut first_sequence = None;
    let mut latest_sequence = None;
    let mut latest_event_type = None;

    for event in events {
        if payload_task_id(&event.payload)? != Some(task.task_id) {
            continue;
        }

        task_event_count += 1;
        first_sequence.get_or_insert(event.sequence);
        latest_sequence = Some(event.sequence);
        latest_event_type = Some(event.event_type.as_str().to_owned());

        match event.event_type {
            EventType::WorkerLaneRequested => {
                worker_workspace_path = payload_optional_string(&event.payload, "workspace_path");
                worker_worktree_path = payload_optional_string(&event.payload, "worktree_path");
            }
            EventType::WorkerLaneWorktreeAllocated => {
                allocated_worktree_path = Some(payload_string(&event.payload, "worktree_path")?);
                session_repo_path = Some(payload_string(&event.payload, "session_repo_path")?);
            }
            EventType::DiffRecorded => {
                diff_repo_path = payload_optional_string(&event.payload, "repo_path");
            }
            EventType::CommitApproved | EventType::CommitRejected => {
                approval_actor = payload_optional_string(&event.payload, "actor");
                approval_reason = payload_optional_string(&event.payload, "reason");
            }
            EventType::CommitStarted | EventType::CommitSucceeded | EventType::CommitFailed => {
                commit_actor = payload_optional_string(&event.payload, "actor");
                commit_repo_path = payload_optional_string(&event.payload, "repo_path");
            }
            _ => {}
        }
    }

    let queue = queue_record
        .map(task_inspect_queue_from_record)
        .or_else(|| task.queue.as_ref().map(task_inspect_queue_from_projection));
    let lease = task.lease.as_ref().map(|lease| TaskInspectLeaseReport {
        lease_id: lease.lease_id,
        worker_id: lease.worker_id.clone(),
        status: lease.status.clone(),
        lease_deadline_ms: lease.lease_deadline_ms,
        reason: lease.reason.clone(),
    });
    let approval = task
        .approval
        .as_ref()
        .map(|approval| TaskInspectApprovalReport {
            state: approval.state.clone(),
            summary: approval.summary.clone(),
            actor: approval_actor.clone(),
            reason: approval_reason.clone(),
            rejection_reason: approval.rejection_reason.clone(),
        });
    let commit = task.commit.as_ref().map(|commit| TaskInspectCommitReport {
        state: commit.state.clone(),
        repo_path: commit_repo_path.clone(),
        message: commit.message.clone(),
        actor: commit_actor.clone(),
        commit_sha: commit.commit_sha.clone(),
        failure_reason: commit.failure_reason.clone(),
    });
    let diff = task.diff.as_ref().map(|diff| TaskInspectDiffReport {
        repo_path: diff_repo_path.clone(),
        files_changed: diff.files_changed,
        insertions: diff.insertions,
        deletions: diff.deletions,
        paths: diff.paths.clone(),
    });

    Ok(TaskInspectReport {
        session_id: task.session_id,
        task_id: task.task_id,
        repo_path: task.repo_path,
        status: task.status,
        input_summary: bounded_text_summary(task.input, 256),
        worker_lane_kind: task.worker_lane_kind,
        context_budget: TaskContextBudgetReport {
            max_bytes: task.context_budget.max_bytes,
            max_files: task.context_budget.max_files,
            max_skill_files: task.context_budget.max_skill_files,
        },
        focus_terms: task.focus_terms,
        max_output_tokens: task.max_output_tokens,
        queue,
        lease,
        approval,
        commit,
        workspace: TaskInspectWorkspaceReport {
            worker_workspace_path,
            worker_worktree_path,
            allocated_worktree_path,
            session_repo_path,
            diff_repo_path,
        },
        diff,
        event_summary: TaskInspectEventSummary {
            event_count: task_event_count,
            first_sequence,
            latest_sequence,
            latest_event_type,
        },
        source_of_truth: "EventLog".to_owned(),
        projection_kind: "task_inspect_report".to_owned(),
    })
}

fn task_inspect_queue_from_record(record: &TaskQueueRecord) -> TaskInspectQueueReport {
    TaskInspectQueueReport {
        status: record.status.clone(),
        worker_id: record.worker_id.clone(),
        lease_id: record.lease_id,
        lease_deadline_ms: record.lease_deadline_ms,
        retry_count: Some(record.retry_count),
        max_retries: Some(record.max_retries),
        stop_reason: record.stop_reason.clone(),
        last_reason: record.last_reason.clone(),
    }
}

fn task_inspect_queue_from_projection(queue: &TaskQueueProjection) -> TaskInspectQueueReport {
    TaskInspectQueueReport {
        status: queue.status.clone(),
        worker_id: None,
        lease_id: None,
        lease_deadline_ms: None,
        retry_count: queue.retry_count,
        max_retries: queue.max_retries,
        stop_reason: queue.stop_reason.clone(),
        last_reason: queue.reason.clone(),
    }
}

fn task_queue_inspect_report_from_records(
    session_id: Option<Uuid>,
    now_ms: i64,
    records: Vec<TaskQueueRecord>,
    task_statuses: &BTreeMap<Uuid, String>,
) -> TaskQueueInspectReport {
    let mut queue_status_counts = BTreeMap::new();
    let mut status_class_counts = BTreeMap::new();
    let mut task_status_counts = BTreeMap::new();
    let mut lease_state_counts = BTreeMap::new();
    let mut active_leased_count = 0;
    let mut expired_looking_leased_count = 0;
    let mut tasks = Vec::new();

    for record in records {
        let (lease_state, status_class) = queue_lease_state_and_class(&record, now_ms);
        if lease_state == "active" {
            active_leased_count += 1;
        } else if lease_state == "expired_looking" {
            expired_looking_leased_count += 1;
        }

        increment_count(&mut queue_status_counts, record.status.clone());
        increment_count(&mut status_class_counts, status_class.clone());
        increment_count(&mut lease_state_counts, lease_state.clone());
        if let Some(task_status) = task_statuses.get(&record.task_id) {
            increment_count(&mut task_status_counts, task_status.clone());
        }

        tasks.push(TaskQueueInspectTaskReport {
            session_id: record.session_id,
            task_id: record.task_id,
            task_status: task_statuses.get(&record.task_id).cloned(),
            queue_status: record.status,
            status_class,
            lease_state,
            worker_id: record.worker_id,
            lease_id: record.lease_id,
            lease_deadline_ms: record.lease_deadline_ms,
            retry_count: record.retry_count,
            max_retries: record.max_retries,
            stop_reason: record.stop_reason,
            last_reason: record.last_reason,
        });
    }

    TaskQueueInspectReport {
        session_id,
        now_ms,
        empty: tasks.is_empty(),
        total_count: tasks.len(),
        active_leased_count,
        expired_looking_leased_count,
        queue_status_counts: status_counts_from_map(queue_status_counts),
        status_class_counts: status_counts_from_map(status_class_counts),
        task_status_counts: status_counts_from_map(task_status_counts),
        lease_state_counts: status_counts_from_map(lease_state_counts),
        tasks,
        source_of_truth: "task_queue+EventLog".to_owned(),
        projection_kind: "task_queue_inspect_report".to_owned(),
    }
}

fn queue_lease_state_and_class(record: &TaskQueueRecord, now_ms: i64) -> (String, String) {
    if record.status != TASK_LEASED_STATE {
        return ("none".to_owned(), record.status.clone());
    }

    match record.lease_deadline_ms {
        Some(deadline) if deadline <= now_ms => (
            "expired_looking".to_owned(),
            "expired_looking_leased".to_owned(),
        ),
        Some(_) => ("active".to_owned(), "active_leased".to_owned()),
        None => (
            "missing_deadline".to_owned(),
            "leased_missing_deadline".to_owned(),
        ),
    }
}

fn increment_count(counts: &mut BTreeMap<String, usize>, key: String) {
    *counts.entry(key).or_insert(0) += 1;
}

fn status_counts_from_map(counts: BTreeMap<String, usize>) -> Vec<TaskQueueInspectStatusCount> {
    counts
        .into_iter()
        .map(|(status, count)| TaskQueueInspectStatusCount { status, count })
        .collect()
}

fn approval_commit_inspect_report_from_events(
    request: ApprovalCommitInspectRequest,
    events: &[EventEnvelope],
) -> HarnessResult<ApprovalCommitInspectReport> {
    let mut diffs = BTreeMap::new();
    let mut workspaces = BTreeMap::new();
    let mut pending = BTreeMap::new();
    let mut decisions = Vec::new();
    let mut commits = Vec::new();

    for event in events {
        let event_task_id = payload_task_id(&event.payload)?;
        if request
            .task_id
            .is_some_and(|task_id| event_task_id != Some(task_id))
        {
            continue;
        }
        let scope_key = event_task_id;

        match event.event_type {
            EventType::WorkerLaneRequested => {
                let workspace = workspaces
                    .entry(scope_key)
                    .or_insert_with(empty_approval_commit_workspace_evidence);
                workspace.worker_workspace_path =
                    payload_optional_string(&event.payload, "workspace_path");
                workspace.worker_worktree_path =
                    payload_optional_string(&event.payload, "worktree_path");
            }
            EventType::WorkerLaneWorktreeAllocated => {
                let workspace = workspaces
                    .entry(scope_key)
                    .or_insert_with(empty_approval_commit_workspace_evidence);
                workspace.allocated_worktree_path =
                    Some(payload_string(&event.payload, "worktree_path")?);
                workspace.session_repo_path =
                    Some(payload_string(&event.payload, "session_repo_path")?);
            }
            EventType::DiffRecorded => {
                let diff = diff_summary_from_payload(&event.payload)?;
                let repo_path = payload_optional_string(&event.payload, "repo_path");
                diffs.insert(
                    scope_key,
                    ApprovalCommitDiffEvidence {
                        repo_path: repo_path.clone(),
                        files_changed: diff.files_changed,
                        insertions: diff.insertions,
                        deletions: diff.deletions,
                        paths: diff.paths,
                    },
                );
                workspaces
                    .entry(scope_key)
                    .or_insert_with(empty_approval_commit_workspace_evidence)
                    .diff_repo_path = repo_path;
            }
            EventType::CommitApprovalPending => {
                pending.insert(
                    scope_key,
                    PendingApprovalInspectEntry {
                        task_id: scope_key,
                        summary: payload_string(&event.payload, "summary")?,
                        diff: diffs.get(&scope_key).cloned(),
                        workspace: workspaces
                            .get(&scope_key)
                            .cloned()
                            .unwrap_or_else(empty_approval_commit_workspace_evidence),
                        event_sequence: event.sequence,
                    },
                );
            }
            EventType::CommitApproved | EventType::CommitRejected => {
                pending.remove(&scope_key);
                let reason = payload_string(&event.payload, "reason")?;
                decisions.push(ApprovalDecisionInspectEntry {
                    task_id: scope_key,
                    state: payload_string(&event.payload, "state")?,
                    actor: payload_optional_string(&event.payload, "actor"),
                    rejection_reason: (event.event_type == EventType::CommitRejected)
                        .then(|| reason.clone()),
                    reason,
                    event_sequence: event.sequence,
                });
            }
            EventType::CommitStarted | EventType::CommitSucceeded | EventType::CommitFailed => {
                commits.push(CommitHandoffInspectEntry {
                    task_id: scope_key,
                    state: payload_string(&event.payload, "state")?,
                    actor: payload_optional_string(&event.payload, "actor"),
                    repo_path: payload_string(&event.payload, "repo_path")?,
                    message: payload_string(&event.payload, "message")?,
                    commit_sha: payload_optional_string(&event.payload, "commit_sha"),
                    failure_reason: payload_optional_string(&event.payload, "failure_reason"),
                    event_sequence: event.sequence,
                });
            }
            _ => {}
        }
    }

    let pending_approvals = pending.into_values().collect::<Vec<_>>();
    Ok(ApprovalCommitInspectReport {
        session_id: request.session_id,
        task_id: request.task_id,
        scope: if request.task_id.is_some() {
            "task".to_owned()
        } else {
            "session".to_owned()
        },
        pending_count: pending_approvals.len(),
        decision_count: decisions.len(),
        commit_count: commits.len(),
        pending_approvals,
        decisions,
        commits,
        event_count: events.len(),
        source_of_truth: "EventLog".to_owned(),
        projection_kind: "approval_commit_inspect_report".to_owned(),
    })
}

fn empty_approval_commit_workspace_evidence() -> ApprovalCommitWorkspaceEvidence {
    ApprovalCommitWorkspaceEvidence {
        worker_workspace_path: None,
        worker_worktree_path: None,
        allocated_worktree_path: None,
        session_repo_path: None,
        diff_repo_path: None,
    }
}

fn worker_lane_diff_inspect_report_from_events(
    request: WorkerLaneDiffInspectRequest,
    events: &[EventEnvelope],
) -> HarnessResult<WorkerLaneDiffInspectReport> {
    let mut session_repo_path = None;
    let mut task_repo_paths = BTreeMap::new();
    for event in events {
        match event.event_type {
            EventType::SessionStarted => {
                session_repo_path = Some(payload_string(&event.payload, "repo_path")?);
            }
            EventType::TaskCreated => {
                task_repo_paths.insert(
                    payload_uuid(&event.payload, "task_id")?,
                    payload_string(&event.payload, "repo_path")?,
                );
            }
            _ => {}
        }
    }

    let mut lanes: BTreeMap<WorkerLaneKey, WorkerLaneInspectEntry> = BTreeMap::new();
    let mut diffs = Vec::new();
    let mut pending_policy_lane_key: Option<WorkerLaneKey> = None;
    let mut latest_lane_by_task_scope: BTreeMap<Option<Uuid>, WorkerLaneKey> = BTreeMap::new();

    for event in events {
        let event_task_id = payload_task_id(&event.payload)?;
        let matches_task_scope = request
            .task_id
            .is_none_or(|task_id| event_task_id == Some(task_id));

        if event.event_type == EventType::PolicyDecided {
            if let Some(lane_key) = pending_policy_lane_key.take() {
                let tool_name = payload_string(&event.payload, "tool_name")?;
                if tool_name == harness_tools::CODEX_WORKER_LANE_TOOL.name
                    && let Some(lane) = lanes.get_mut(&lane_key)
                {
                    lane.policy = Some(WorkerLanePolicyInspectEntry {
                        tool_name,
                        decision: payload_string(&event.payload, "decision")?,
                        reason: payload_string(&event.payload, "reason")?,
                        event_sequence: event.sequence,
                    });
                }
            }
            continue;
        }

        if !matches_task_scope {
            continue;
        }

        match event.event_type {
            EventType::WorkerLaneRequested => {
                let lane_id = payload_string(&event.payload, "lane_id")?;
                let lane_kind = payload_string(&event.payload, "lane_kind")?;
                let lane_key = (event_task_id, lane_id.clone());
                let task_repo_path = event_task_id
                    .and_then(|task_id| task_repo_paths.get(&task_id).map(ToOwned::to_owned));
                let lane = worker_lane_entry_mut(
                    &mut lanes,
                    event_task_id,
                    lane_id,
                    lane_kind,
                    session_repo_path.clone(),
                    task_repo_path,
                );
                let budget = event
                    .payload
                    .get("budget")
                    .ok_or_else(|| HarnessError::new("worker lane budget is missing"))?;
                lane.request_status = Some("requested".to_owned());
                lane.workspace.requested_workspace_path =
                    Some(payload_string(&event.payload, "workspace_path")?);
                lane.workspace.requested_worktree_path =
                    payload_optional_string(&event.payload, "worktree_path");
                lane.request = Some(WorkerLaneRequestInspectEntry {
                    task: bounded_text_summary(
                        payload_string(&event.payload, "task")?,
                        request.payload_limit_bytes,
                    ),
                    workspace_path: payload_string(&event.payload, "workspace_path")?,
                    worktree_path: payload_optional_string(&event.payload, "worktree_path"),
                    timeout_ms: payload_u64(&event.payload, "timeout_ms")?,
                    cancellation_requested: payload_bool(&event.payload, "cancellation_requested")?,
                    budget: WorkerLaneBudgetInspect {
                        max_prompt_tokens: payload_usize(budget, "max_prompt_tokens")?,
                        max_output_tokens: payload_usize(budget, "max_output_tokens")?,
                        max_stdout_bytes: payload_usize(budget, "max_stdout_bytes")?,
                    },
                    event_sequence: event.sequence,
                });
                pending_policy_lane_key = Some(lane_key.clone());
                latest_lane_by_task_scope.insert(event_task_id, lane_key);
            }
            EventType::WorkerLaneStateChanged => {
                let lane_id = payload_string(&event.payload, "lane_id")?;
                let lane_kind = payload_string(&event.payload, "lane_kind")?;
                let task_repo_path = event_task_id
                    .and_then(|task_id| task_repo_paths.get(&task_id).map(ToOwned::to_owned));
                let lane = worker_lane_entry_mut(
                    &mut lanes,
                    event_task_id,
                    lane_id,
                    lane_kind,
                    session_repo_path.clone(),
                    task_repo_path,
                );
                let to_state = payload_string(&event.payload, "to_state")?;
                let reason = payload_string(&event.payload, "reason")?;
                lane.states.push(WorkerLaneStateInspectEntry {
                    from_state: payload_optional_string(&event.payload, "from_state"),
                    to_state: to_state.clone(),
                    reason: reason.clone(),
                    event_sequence: event.sequence,
                });
                lane.current_state = Some(to_state.clone());
                lane.is_running = to_state == WorkerLaneStatus::Running.as_str();
                if worker_lane_state_is_terminal(&to_state) {
                    lane.is_terminal = true;
                    lane.terminal_state = Some(to_state.clone());
                    lane.terminal_reason = Some(reason.clone());
                }
                match to_state.as_str() {
                    "failed" | "rejected" => lane.failure_reason = Some(reason),
                    "timed_out" => lane.timeout_reason = Some(reason),
                    "cancelled" => lane.cancellation_reason = Some(reason),
                    _ => {}
                }
            }
            EventType::WorkerLaneWorktreeAllocated => {
                let lane_id = payload_string(&event.payload, "lane_id")?;
                let lane_kind = payload_string(&event.payload, "lane_kind")?;
                let task_repo_path = event_task_id
                    .and_then(|task_id| task_repo_paths.get(&task_id).map(ToOwned::to_owned));
                let lane = worker_lane_entry_mut(
                    &mut lanes,
                    event_task_id,
                    lane_id,
                    lane_kind,
                    session_repo_path.clone(),
                    task_repo_path,
                );
                lane.workspace.allocated_worktree_path =
                    Some(payload_string(&event.payload, "worktree_path")?);
                lane.workspace.session_repo_path =
                    Some(payload_string(&event.payload, "session_repo_path")?);
                lane.workspace.base_ref = Some(payload_string(&event.payload, "base_ref")?);
            }
            EventType::WorkerLaneObservationRecorded => {
                let lane_id = payload_string(&event.payload, "lane_id")?;
                let lane_kind = payload_string(&event.payload, "lane_kind")?;
                let task_repo_path = event_task_id
                    .and_then(|task_id| task_repo_paths.get(&task_id).map(ToOwned::to_owned));
                let lane = worker_lane_entry_mut(
                    &mut lanes,
                    event_task_id,
                    lane_id,
                    lane_kind,
                    session_repo_path.clone(),
                    task_repo_path,
                );
                let status = payload_string(&event.payload, "status")?;
                let stderr = payload_string(&event.payload, "stderr")?;
                lane.observation = Some(WorkerLaneObservationInspectEntry {
                    status: status.clone(),
                    exit_code: payload_i32_optional(&event.payload, "exit_code")?,
                    stdout: bounded_text_summary(
                        payload_string(&event.payload, "stdout")?,
                        request.payload_limit_bytes,
                    ),
                    stderr: bounded_text_summary(stderr.clone(), request.payload_limit_bytes),
                    duration_ms: payload_u64(&event.payload, "duration_ms")?,
                    prompt_tokens: payload_usize_optional(&event.payload, "prompt_tokens")?,
                    completion_tokens: payload_usize_optional(&event.payload, "completion_tokens")?,
                    usage_confidence: payload_string(&event.payload, "usage_confidence")?,
                    skipped_or_unavailable_reason: skipped_or_unavailable_reason(&status, &stderr),
                    event_sequence: event.sequence,
                });
            }
            EventType::DiffRecorded => {
                let diff = diff_summary_from_payload(&event.payload)?;
                let repo_path = payload_optional_string(&event.payload, "repo_path");
                if let Some(lane_key) = latest_lane_by_task_scope.get(&event_task_id)
                    && let Some(lane) = lanes.get_mut(lane_key)
                {
                    lane.workspace.diff_repo_path = repo_path.clone();
                }
                diffs.push(WorkerLaneDiffEvidenceInspectEntry {
                    task_id: event_task_id,
                    repo_path,
                    files_changed: diff.files_changed,
                    insertions: diff.insertions,
                    deletions: diff.deletions,
                    paths: diff.paths,
                    git_status: bounded_text_summary(
                        payload_optional_string(&event.payload, "git_status").unwrap_or_default(),
                        request.payload_limit_bytes,
                    ),
                    git_diff: bounded_text_summary(
                        payload_optional_string(&event.payload, "git_diff").unwrap_or_default(),
                        request.payload_limit_bytes,
                    ),
                    event_sequence: event.sequence,
                });
            }
            _ => {}
        }
    }

    let lanes = lanes.into_values().collect::<Vec<_>>();
    Ok(WorkerLaneDiffInspectReport {
        session_id: request.session_id,
        task_id: request.task_id,
        scope: if request.task_id.is_some() {
            "task".to_owned()
        } else {
            "session".to_owned()
        },
        lane_count: lanes.len(),
        diff_count: diffs.len(),
        event_count: events.len(),
        payload_limit_bytes: request.payload_limit_bytes,
        lanes,
        diffs,
        source_of_truth: "EventLog".to_owned(),
        projection_kind: "worker_lane_diff_inspect_report".to_owned(),
    })
}

type WorkerLaneKey = (Option<Uuid>, String);

fn worker_lane_entry_mut(
    lanes: &mut BTreeMap<WorkerLaneKey, WorkerLaneInspectEntry>,
    task_id: Option<Uuid>,
    lane_id: String,
    lane_kind: String,
    session_repo_path: Option<String>,
    task_repo_path: Option<String>,
) -> &mut WorkerLaneInspectEntry {
    lanes
        .entry((task_id, lane_id.clone()))
        .or_insert_with(|| WorkerLaneInspectEntry {
            task_id,
            lane_id,
            lane_kind,
            request_status: None,
            request: None,
            policy: None,
            states: Vec::new(),
            current_state: None,
            is_running: false,
            is_terminal: false,
            terminal_state: None,
            terminal_reason: None,
            failure_reason: None,
            timeout_reason: None,
            cancellation_reason: None,
            workspace: WorkerLaneWorkspaceInspectEntry {
                requested_workspace_path: None,
                requested_worktree_path: None,
                allocated_worktree_path: None,
                session_repo_path,
                task_repo_path,
                diff_repo_path: None,
                base_ref: None,
            },
            observation: None,
        })
}

fn worker_lane_state_is_terminal(state: &str) -> bool {
    matches!(
        state,
        "succeeded" | "failed" | "cancelled" | "timed_out" | "rejected"
    )
}

fn skipped_or_unavailable_reason(status: &str, stderr: &str) -> Option<String> {
    let normalized_status = status.to_ascii_lowercase();
    let normalized_stderr = stderr.to_ascii_lowercase();
    if normalized_status.contains("skipped") || normalized_stderr.contains("unavailable") {
        Some(stderr.to_owned())
    } else {
        None
    }
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
            EventType::TaskEnqueued | EventType::TaskRetryQueued | EventType::TaskRetryStopped => {
                let status = payload_string(&event.payload, "status")?;
                task.status = status.clone();
                task.queue = Some(TaskQueueProjection {
                    status,
                    reason: payload_optional_string(&event.payload, "reason"),
                    retry_count: payload_i32_optional(&event.payload, "retry_count")?,
                    max_retries: payload_i32_optional(&event.payload, "max_retries")?,
                    stop_reason: payload_optional_string(&event.payload, "stop_reason"),
                });
            }
            EventType::TaskLeaseAcquired
            | EventType::TaskLeaseRenewed
            | EventType::TaskLeaseExpired
            | EventType::TaskLeaseCompleted
            | EventType::TaskLeaseFailed
            | EventType::TaskLeaseCancelled => {
                let status = payload_string(&event.payload, "status")?;
                let reason = payload_optional_string(&event.payload, "reason");
                if event.event_type != EventType::TaskLeaseCompleted || task.approval.is_none() {
                    task.status = status.clone();
                }
                if event.event_type != EventType::TaskLeaseExpired {
                    task.queue = Some(TaskQueueProjection {
                        status: status.clone(),
                        reason: reason.clone(),
                        retry_count: payload_i32_optional(&event.payload, "retry_count")?,
                        max_retries: payload_i32_optional(&event.payload, "max_retries")?,
                        stop_reason: payload_optional_string(&event.payload, "stop_reason"),
                    });
                }
                task.lease = Some(TaskLeaseProjection {
                    lease_id: payload_uuid(&event.payload, "lease_id")?,
                    worker_id: payload_string(&event.payload, "worker_id")?,
                    status,
                    lease_deadline_ms: payload_i64_optional(&event.payload, "lease_deadline_ms"),
                    reason,
                });
            }
            EventType::WorkerLaneRequested => {
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
        queue: None,
        lease: None,
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

fn model_provider_selection(provider: &RuntimeModelProvider) -> ModelProviderSelection {
    match provider {
        RuntimeModelProvider::DeterministicFake => ModelProviderSelection::DeterministicFake,
        RuntimeModelProvider::OpenAiCompatible {
            model_id,
            api_key_env,
        } => ModelProviderSelection::OpenAiCompatible {
            model_id: model_id.clone(),
            api_key_available: env::var_os(api_key_env).is_some(),
        },
    }
}

fn sanitize_provider_error_message(message: &str) -> String {
    let mut sanitized = message.to_owned();

    for (key, value) in env::vars() {
        let key_upper = key.to_ascii_uppercase();
        let looks_sensitive = key_upper.contains("KEY")
            || key_upper.contains("TOKEN")
            || key_upper.contains("SECRET")
            || key_upper.contains("PASSWORD");

        if looks_sensitive && value.len() >= 4 {
            sanitized = sanitized.replace(&value, "[REDACTED]");
        }
    }

    sanitized
}

fn model_task_summary(task: &str) -> String {
    format!("redacted_task_chars={}", task.chars().count())
}

fn model_patch_evidence_payload(patch: &FilePatchProposal) -> FilePatchPayload {
    FilePatchPayload::new(patch.path.clone(), None, "[REDACTED_MODEL_PATCH_PROPOSAL]")
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
    approval_projection_from_events_scoped(session_id, events, None)
}

fn approval_projection_from_events_scoped(
    session_id: Uuid,
    events: &[EventEnvelope],
    task_id: Option<Uuid>,
) -> HarnessResult<ApprovalProjection> {
    let mut diff = None;
    let mut state = None;
    let mut summary = String::new();
    let mut rejection_reason = None;

    for event in events {
        if !event_matches_task_scope(event, task_id)? {
            continue;
        }

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
    commit_handoff_projection_from_events_scoped(session_id, events, None)
}

fn commit_handoff_projection_from_events_scoped(
    session_id: Uuid,
    events: &[EventEnvelope],
    task_id: Option<Uuid>,
) -> HarnessResult<CommitHandoffProjection> {
    let mut projection = None;

    for event in events {
        if !event_matches_task_scope(event, task_id)? {
            continue;
        }

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
    existing_commit_handoff_state_from_events_scoped(events, None)
}

fn existing_commit_handoff_state_from_events_scoped(
    events: &[EventEnvelope],
    task_id: Option<Uuid>,
) -> HarnessResult<Option<String>> {
    let mut state = None;

    for event in events {
        if !event_matches_task_scope(event, task_id)? {
            continue;
        }

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
    commit_repo_path_from_events_scoped(events, None)
}

fn commit_repo_path_from_events_scoped(
    events: &[EventEnvelope],
    task_id: Option<Uuid>,
) -> HarnessResult<String> {
    let mut session_repo_path = None;
    let mut task_repo_path = None;
    let mut latest_worktree_path = None;
    let mut diff_repo_path = None;

    for event in events {
        match event.event_type {
            EventType::SessionStarted => {
                if task_id.is_none() {
                    session_repo_path = Some(payload_string(&event.payload, "repo_path")?);
                }
            }
            EventType::TaskCreated => {
                if event_matches_task_scope(event, task_id)? {
                    task_repo_path = Some(payload_string(&event.payload, "repo_path")?);
                }
            }
            EventType::WorkerLaneWorktreeAllocated => {
                if event_matches_task_scope(event, task_id)? {
                    latest_worktree_path = Some(payload_string(&event.payload, "worktree_path")?);
                }
            }
            EventType::DiffRecorded if event_matches_task_scope(event, task_id)? => {
                diff_repo_path = event
                    .payload
                    .get("repo_path")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned)
                    .or_else(|| latest_worktree_path.clone())
                    .or_else(|| task_repo_path.clone())
                    .or_else(|| session_repo_path.clone());
            }
            _ => {}
        }
    }

    diff_repo_path.ok_or_else(|| HarnessError::new("diff repository path was not recorded"))
}

fn event_matches_task_scope(event: &EventEnvelope, task_id: Option<Uuid>) -> HarnessResult<bool> {
    payload_task_id(&event.payload).map(|payload_task_id| payload_task_id == task_id)
}

fn effective_queued_worker_lease_duration_ms(
    requested_lease_duration_ms: u64,
    worker_timeout_ms: u64,
) -> u64 {
    requested_lease_duration_ms
        .max(worker_timeout_ms.saturating_add(QUEUED_WORKER_LEASE_TIMEOUT_MARGIN_MS))
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

fn payload_uuid(payload: &serde_json::Value, key: &str) -> HarnessResult<Uuid> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .ok_or_else(|| HarnessError::new(format!("payload {key} is missing")))
        .and_then(|value| {
            Uuid::parse_str(value).map_err(|error| HarnessError::new(error.to_string()))
        })
}

fn payload_task_id(payload: &serde_json::Value) -> HarnessResult<Option<Uuid>> {
    payload
        .get("task_id")
        .and_then(|value| value.as_str())
        .map(|value| Uuid::parse_str(value).map_err(|error| HarnessError::new(error.to_string())))
        .transpose()
}

fn payload_i64_optional(payload: &serde_json::Value, key: &str) -> Option<i64> {
    payload.get(key).and_then(|value| value.as_i64())
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

fn payload_bool(payload: &serde_json::Value, key: &str) -> HarnessResult<bool> {
    payload
        .get(key)
        .and_then(|value| value.as_bool())
        .ok_or_else(|| HarnessError::new(format!("payload {key} is missing")))
}

fn task_lease_payload_from_record(record: &TaskQueueRecord) -> HarnessResult<TaskLeasePayload> {
    Ok(TaskLeasePayload::new(
        record.task_id,
        record
            .lease_id
            .ok_or_else(|| HarnessError::new("task lease id was not recorded"))?,
        record
            .worker_id
            .as_deref()
            .ok_or_else(|| HarnessError::new("task lease worker id was not recorded"))?,
        record.status.as_str(),
        record.lease_deadline_ms,
        record
            .last_reason
            .as_deref()
            .unwrap_or("task lease updated"),
    ))
    .map(|payload| {
        payload.with_retry_state(
            record.retry_count,
            record.max_retries,
            record.stop_reason.clone(),
        )
    })
}

fn expired_lease_payload_from_record(
    record: &TaskLeaseExpirationRecord,
) -> HarnessResult<TaskLeasePayload> {
    let status = if record.queue.status == TASK_STOPPED_STATE {
        TASK_STOPPED_STATE
    } else {
        TASK_LEASE_EXPIRED_STATE
    };

    Ok(TaskLeasePayload::new(
        record.queue.task_id,
        record.expired_lease_id,
        record.expired_worker_id.clone(),
        status,
        Some(record.expired_deadline_ms),
        record
            .queue
            .last_reason
            .as_deref()
            .unwrap_or("task lease expired"),
    )
    .with_retry_state(
        record.queue.retry_count,
        record.queue.max_retries,
        record.queue.stop_reason.clone(),
    ))
}

fn task_queue_payload_from_record(record: &TaskQueueRecord) -> HarnessResult<TaskQueuePayload> {
    Ok(TaskQueuePayload::new(
        record.task_id,
        record.status.as_str(),
        record
            .last_reason
            .as_deref()
            .unwrap_or("task queue updated"),
    )
    .with_retry_state(
        record.retry_count,
        record.max_retries,
        record.stop_reason.clone(),
    ))
}

fn current_time_ms() -> HarnessResult<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| HarnessError::new(error.to_string()))?;
    i64::try_from(duration.as_millis())
        .map_err(|_| HarnessError::new("system time is out of range"))
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

fn payload_usize_optional(payload: &serde_json::Value, key: &str) -> HarnessResult<Option<usize>> {
    payload
        .get(key)
        .and_then(|value| value.as_u64())
        .map(|value| {
            usize::try_from(value)
                .map_err(|_| HarnessError::new(format!("payload {key} is out of range")))
        })
        .transpose()
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

    let git_repo_path = git_cli_path(repo_path);
    let git_worktree_path = git_cli_path(&worktree_path);

    let output = Command::new("git")
        .arg("-C")
        .arg(git_repo_path)
        .arg("worktree")
        .arg("add")
        .arg("--detach")
        .arg(git_worktree_path)
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

fn git_cli_path(path: &Path) -> PathBuf {
    let path_text = path.display().to_string();

    if let Some(stripped) = path_text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{stripped}"));
    }

    if let Some(stripped) = path_text.strip_prefix(r"\\?\") {
        return PathBuf::from(stripped);
    }

    PathBuf::from(path)
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
    task_id: Option<Uuid>,
) -> WorkerLaneRequestPayload {
    WorkerLaneRequestPayload {
        task_id,
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
    task_id: Option<Uuid>,
) -> WorkerLaneObservationPayload {
    WorkerLaneObservationPayload {
        task_id,
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
    fn architecture_report_contains_required_nodes_and_boundaries() {
        let report = Runtime::architecture_report();
        let node_ids = report
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>();
        let section_ids = report
            .sections
            .iter()
            .map(|section| section.id.as_str())
            .collect::<Vec<_>>();
        let edge_ids = report
            .edges
            .iter()
            .map(|edge| edge.id.as_str())
            .collect::<Vec<_>>();
        let non_goal_ids = report
            .non_goals
            .iter()
            .map(|non_goal| non_goal.id.as_str())
            .collect::<Vec<_>>();

        for required in [
            "cli_api",
            "runtime",
            "policy_gate",
            "tool_runtime",
            "eventlog",
            "postgres_task_queue",
            "local_governed_codex_cli_worker",
            "task_worktree",
            "approval_state_machine",
            "commit_handoff",
            "task_lease",
            "vertical_acceptance_path",
        ] {
            assert!(node_ids.contains(&required), "missing node {required}");
        }

        assert_eq!(
            report.boundary_adrs,
            vec!["ADR-0001", "ADR-0002", "ADR-0004", "ADR-0007"]
        );
        assert!(section_ids.contains(&"governance"));
        assert!(edge_ids.contains(&"runtime_to_eventlog"));
        assert!(edge_ids.contains(&"policy_to_tool_runtime"));
        assert!(non_goal_ids.contains(&"web_ui"));
        assert!(non_goal_ids.contains(&"replace_eventlog"));
        assert!(
            report
                .nodes
                .iter()
                .any(|node| node.glossary_terms.iter().any(|term| term == "Task Lease"))
        );
        assert!(report.nodes.iter().any(|node| {
            node.glossary_terms
                .iter()
                .any(|term| term == "Commit Handoff")
        }));
        assert_eq!(report.projection_kind, "runtime_architecture_report");
    }

    #[test]
    fn data_flow_report_lists_required_steps_in_order() {
        let report = Runtime::data_flow_report();
        let step_ids = report
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            step_ids,
            vec![
                "task_creation",
                "context_compile",
                "enqueue",
                "lease",
                "worker_lane_request",
                "policy_decision",
                "worktree_allocation",
                "worker_observation",
                "diff_recorded",
                "pending_approval",
                "approval_decision",
                "commit_started",
                "commit_finished",
            ]
        );
        assert_eq!(report.steps[0].event_types, vec!["task.created"]);
        assert_eq!(report.steps[5].component_id, "policy_gate");
        assert_eq!(
            report.steps[10].event_types,
            vec!["commit.approved", "commit.rejected"]
        );
        assert_eq!(
            report.steps.last().expect("last step").event_types,
            vec!["commit.succeeded", "commit.failed"]
        );
        assert_eq!(report.edges.len(), report.steps.len() - 1);
        assert_eq!(report.edges[0].from, "task_creation");
        assert_eq!(report.edges[0].to, "context_compile");
        assert_eq!(report.projection_kind, "runtime_data_flow_report");
    }

    #[test]
    fn storage_report_explains_eventlog_queue_and_projection_boundaries() {
        let report = Runtime::storage_report();
        let table_ids = report
            .tables
            .iter()
            .map(|table| table.id.as_str())
            .collect::<Vec<_>>();
        let projection_ids = report
            .projections
            .iter()
            .map(|projection| projection.id.as_str())
            .collect::<Vec<_>>();

        assert!(table_ids.contains(&"event_log"));
        assert!(table_ids.contains(&"task_queue"));
        let event_log = report
            .tables
            .iter()
            .find(|table| table.id == "event_log")
            .expect("event_log table report");
        assert_eq!(event_log.table_name, "harness_runtime.event_log");
        assert_eq!(event_log.role, "append_only_source_of_truth");
        assert!(event_log.ownership_boundary.contains("Runtime appends"));

        let task_queue = report
            .tables
            .iter()
            .find(|table| table.id == "task_queue")
            .expect("task_queue table report");
        assert_eq!(task_queue.table_name, "harness_runtime.task_queue");
        assert_eq!(task_queue.role, "runtime_scheduling_state");
        assert!(
            task_queue
                .derived_from
                .as_deref()
                .is_some_and(|source| source.contains("EventLog"))
        );
        assert!(projection_ids.contains(&"task_projection"));
        assert!(projection_ids.contains(&"approval_commit_projection"));
        assert!(
            report
                .non_goals
                .iter()
                .any(|non_goal| non_goal.id == "distributed_scheduling")
        );
        assert_eq!(report.projection_kind, "runtime_storage_report");
    }

    #[test]
    fn queued_worker_lease_duration_covers_worker_timeout() {
        assert_eq!(
            effective_queued_worker_lease_duration_ms(60_000, 30_000),
            60_000
        );
        assert_eq!(
            effective_queued_worker_lease_duration_ms(60_000, 300_000),
            301_000
        );
        assert_eq!(
            effective_queued_worker_lease_duration_ms(u64::MAX, 300_000),
            u64::MAX
        );
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
        let lease_id = Uuid::new_v4();
        let enqueued = NewEvent::task_enqueued(
            session_id,
            TaskQueuePayload::new(
                task_id,
                TASK_QUEUED_STATE,
                "task enqueued for worker execution",
            ),
        )
        .expect("task enqueued event");
        let lease_acquired = NewEvent::task_lease_acquired(
            session_id,
            TaskLeasePayload::new(
                task_id,
                lease_id,
                "worker-1",
                TASK_LEASED_STATE,
                Some(123_456),
                "task lease acquired",
            ),
        )
        .expect("task lease acquired event");
        let lease_completed = NewEvent::task_lease_completed(
            session_id,
            TaskLeasePayload::new(
                task_id,
                lease_id,
                "worker-1",
                TASK_COMPLETED_STATE,
                Some(123_456),
                "worker completed task",
            ),
        )
        .expect("task lease completed event");
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
            event_envelope(3, enqueued),
            event_envelope(4, lease_acquired),
            event_envelope(5, lease_completed),
            event_envelope(6, diff),
            event_envelope(7, approval),
            event_envelope(8, commit),
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
        let queue = tasks[0].queue.as_ref().expect("task queue slot");
        assert_eq!(queue.status, TASK_COMPLETED_STATE);
        assert_eq!(queue.reason.as_deref(), Some("worker completed task"));
        let lease = tasks[0].lease.as_ref().expect("task lease slot");
        assert_eq!(lease.lease_id, lease_id);
        assert_eq!(lease.worker_id, "worker-1");
        assert_eq!(lease.status, TASK_COMPLETED_STATE);
        assert_eq!(lease.lease_deadline_ms, Some(123_456));
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
        assert_eq!(tasks[0].event_count, 8);
    }

    #[test]
    fn task_projection_replays_lease_recovery_without_database() {
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let first_lease_id = Uuid::new_v4();
        let second_lease_id = Uuid::new_v4();
        let task_created = NewEvent::task_created(
            session_id,
            TaskCreatedPayload::new(
                task_id,
                "C:/repo",
                "draft a recoverable task",
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
        let enqueued = NewEvent::task_enqueued(
            session_id,
            TaskQueuePayload::new(
                task_id,
                TASK_QUEUED_STATE,
                "task enqueued for worker execution",
            )
            .with_retry_state(0, 1, None),
        )
        .expect("task enqueued event");
        let first_lease = NewEvent::task_lease_acquired(
            session_id,
            TaskLeasePayload::new(
                task_id,
                first_lease_id,
                "worker-1",
                TASK_LEASED_STATE,
                Some(100),
                "task lease acquired",
            )
            .with_retry_state(0, 1, None),
        )
        .expect("first lease event");
        let renewed = NewEvent::task_lease_renewed(
            session_id,
            TaskLeasePayload::new(
                task_id,
                first_lease_id,
                "worker-1",
                TASK_LEASED_STATE,
                Some(200),
                "task lease renewed",
            )
            .with_retry_state(0, 1, None),
        )
        .expect("renewed event");
        let expired = NewEvent::task_lease_expired(
            session_id,
            TaskLeasePayload::new(
                task_id,
                first_lease_id,
                "worker-1",
                TASK_LEASE_EXPIRED_STATE,
                Some(200),
                "task lease expired; task released for retry",
            )
            .with_retry_state(1, 1, Some(TASK_STOP_REASON_LEASE_EXPIRED.to_owned())),
        )
        .expect("expired event");
        let retry_queued = NewEvent::task_retry_queued(
            session_id,
            TaskQueuePayload::new(
                task_id,
                TASK_RETRY_QUEUED_STATE,
                "task lease expired; task released for retry",
            )
            .with_retry_state(1, 1, Some(TASK_STOP_REASON_LEASE_EXPIRED.to_owned())),
        )
        .expect("retry queued event");
        let second_lease = NewEvent::task_lease_acquired(
            session_id,
            TaskLeasePayload::new(
                task_id,
                second_lease_id,
                "worker-2",
                TASK_LEASED_STATE,
                Some(300),
                "task lease acquired",
            )
            .with_retry_state(1, 1, None),
        )
        .expect("second lease event");
        let stopped_lease = NewEvent::task_lease_expired(
            session_id,
            TaskLeasePayload::new(
                task_id,
                second_lease_id,
                "worker-2",
                TASK_STOPPED_STATE,
                Some(300),
                "task lease expired; max retries exceeded",
            )
            .with_retry_state(
                1,
                1,
                Some(TASK_STOP_REASON_MAX_RETRIES_EXCEEDED.to_owned()),
            ),
        )
        .expect("stopped lease event");
        let retry_stopped = NewEvent::task_retry_stopped(
            session_id,
            TaskQueuePayload::new(
                task_id,
                TASK_STOPPED_STATE,
                "task lease expired; max retries exceeded",
            )
            .with_retry_state(
                1,
                1,
                Some(TASK_STOP_REASON_MAX_RETRIES_EXCEEDED.to_owned()),
            ),
        )
        .expect("retry stopped event");
        let events = vec![
            event_envelope(1, task_created),
            event_envelope(2, enqueued),
            event_envelope(3, first_lease),
            event_envelope(4, renewed),
            event_envelope(5, expired),
            event_envelope(6, retry_queued),
            event_envelope(7, second_lease),
            event_envelope(8, stopped_lease),
            event_envelope(9, retry_stopped),
        ];

        let tasks = task_projections_from_events(session_id, &events).expect("task projection");

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, TASK_STOPPED_STATE);
        let queue = tasks[0].queue.as_ref().expect("queue slot");
        assert_eq!(queue.status, TASK_STOPPED_STATE);
        assert_eq!(queue.retry_count, Some(1));
        assert_eq!(queue.max_retries, Some(1));
        assert_eq!(
            queue.stop_reason.as_deref(),
            Some(TASK_STOP_REASON_MAX_RETRIES_EXCEEDED)
        );
        let lease = tasks[0].lease.as_ref().expect("lease slot");
        assert_eq!(lease.lease_id, second_lease_id);
        assert_eq!(lease.worker_id, "worker-2");
        assert_eq!(lease.status, TASK_STOPPED_STATE);
        assert_eq!(lease.lease_deadline_ms, Some(300));
        assert_eq!(tasks[0].event_count, 9);
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
