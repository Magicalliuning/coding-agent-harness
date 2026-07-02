use harness_core::{HarnessError, HarnessResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const EVENTLOG_SCHEMA_VERSION: u16 = 1;

pub const SESSION_STARTED_EVENT: &str = "session.started";
pub const TASK_CREATED_EVENT: &str = "task.created";
pub const TASK_ENQUEUED_EVENT: &str = "task.enqueued";
pub const TASK_LEASE_ACQUIRED_EVENT: &str = "task.lease_acquired";
pub const TASK_LEASE_RENEWED_EVENT: &str = "task.lease_renewed";
pub const TASK_LEASE_EXPIRED_EVENT: &str = "task.lease_expired";
pub const TASK_LEASE_COMPLETED_EVENT: &str = "task.lease_completed";
pub const TASK_LEASE_FAILED_EVENT: &str = "task.lease_failed";
pub const TASK_LEASE_CANCELLED_EVENT: &str = "task.lease_cancelled";
pub const TASK_RETRY_QUEUED_EVENT: &str = "task.retry_queued";
pub const TASK_RETRY_STOPPED_EVENT: &str = "task.retry_stopped";
pub const TOOL_CALL_INTENDED_EVENT: &str = "tool.call_intended";
pub const POLICY_DECIDED_EVENT: &str = "policy.decided";
pub const TOOL_OBSERVATION_RECORDED_EVENT: &str = "tool.observation_recorded";
pub const CONTEXT_COMPILED_EVENT: &str = "context.compiled";
pub const MODEL_REQUEST_RECORDED_EVENT: &str = "model.request_recorded";
pub const MODEL_DECISION_RECORDED_EVENT: &str = "model.decision_recorded";
pub const DIFF_RECORDED_EVENT: &str = "diff.recorded";
pub const COMMIT_APPROVAL_PENDING_EVENT: &str = "commit.approval_pending";
pub const COMMIT_APPROVED_EVENT: &str = "commit.approved";
pub const COMMIT_REJECTED_EVENT: &str = "commit.rejected";
pub const COMMIT_STARTED_EVENT: &str = "commit.started";
pub const COMMIT_SUCCEEDED_EVENT: &str = "commit.succeeded";
pub const COMMIT_FAILED_EVENT: &str = "commit.failed";
pub const RECOVERY_FAILURE_CLASSIFIED_EVENT: &str = "recovery.failure_classified";
pub const RECOVERY_PLAN_RECORDED_EVENT: &str = "recovery.plan_recorded";
pub const RECOVERY_REPAIR_ATTEMPTED_EVENT: &str = "recovery.repair_attempted";
pub const RECOVERY_STOPPED_EVENT: &str = "recovery.stopped";
pub const WORKER_LANE_REQUESTED_EVENT: &str = "worker_lane.requested";
pub const WORKER_LANE_STATE_CHANGED_EVENT: &str = "worker_lane.state_changed";
pub const WORKER_LANE_WORKTREE_ALLOCATED_EVENT: &str = "worker_lane.worktree_allocated";
pub const WORKER_LANE_OBSERVATION_RECORDED_EVENT: &str = "worker_lane.observation_recorded";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    SessionStarted,
    TaskCreated,
    TaskEnqueued,
    TaskLeaseAcquired,
    TaskLeaseRenewed,
    TaskLeaseExpired,
    TaskLeaseCompleted,
    TaskLeaseFailed,
    TaskLeaseCancelled,
    TaskRetryQueued,
    TaskRetryStopped,
    ToolCallIntended,
    PolicyDecided,
    ToolObservationRecorded,
    ContextCompiled,
    ModelRequestRecorded,
    ModelDecisionRecorded,
    DiffRecorded,
    CommitApprovalPending,
    CommitApproved,
    CommitRejected,
    CommitStarted,
    CommitSucceeded,
    CommitFailed,
    RecoveryFailureClassified,
    RecoveryPlanRecorded,
    RecoveryRepairAttempted,
    RecoveryStopped,
    WorkerLaneRequested,
    WorkerLaneStateChanged,
    WorkerLaneWorktreeAllocated,
    WorkerLaneObservationRecorded,
}

impl EventType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStarted => SESSION_STARTED_EVENT,
            Self::TaskCreated => TASK_CREATED_EVENT,
            Self::TaskEnqueued => TASK_ENQUEUED_EVENT,
            Self::TaskLeaseAcquired => TASK_LEASE_ACQUIRED_EVENT,
            Self::TaskLeaseRenewed => TASK_LEASE_RENEWED_EVENT,
            Self::TaskLeaseExpired => TASK_LEASE_EXPIRED_EVENT,
            Self::TaskLeaseCompleted => TASK_LEASE_COMPLETED_EVENT,
            Self::TaskLeaseFailed => TASK_LEASE_FAILED_EVENT,
            Self::TaskLeaseCancelled => TASK_LEASE_CANCELLED_EVENT,
            Self::TaskRetryQueued => TASK_RETRY_QUEUED_EVENT,
            Self::TaskRetryStopped => TASK_RETRY_STOPPED_EVENT,
            Self::ToolCallIntended => TOOL_CALL_INTENDED_EVENT,
            Self::PolicyDecided => POLICY_DECIDED_EVENT,
            Self::ToolObservationRecorded => TOOL_OBSERVATION_RECORDED_EVENT,
            Self::ContextCompiled => CONTEXT_COMPILED_EVENT,
            Self::ModelRequestRecorded => MODEL_REQUEST_RECORDED_EVENT,
            Self::ModelDecisionRecorded => MODEL_DECISION_RECORDED_EVENT,
            Self::DiffRecorded => DIFF_RECORDED_EVENT,
            Self::CommitApprovalPending => COMMIT_APPROVAL_PENDING_EVENT,
            Self::CommitApproved => COMMIT_APPROVED_EVENT,
            Self::CommitRejected => COMMIT_REJECTED_EVENT,
            Self::CommitStarted => COMMIT_STARTED_EVENT,
            Self::CommitSucceeded => COMMIT_SUCCEEDED_EVENT,
            Self::CommitFailed => COMMIT_FAILED_EVENT,
            Self::RecoveryFailureClassified => RECOVERY_FAILURE_CLASSIFIED_EVENT,
            Self::RecoveryPlanRecorded => RECOVERY_PLAN_RECORDED_EVENT,
            Self::RecoveryRepairAttempted => RECOVERY_REPAIR_ATTEMPTED_EVENT,
            Self::RecoveryStopped => RECOVERY_STOPPED_EVENT,
            Self::WorkerLaneRequested => WORKER_LANE_REQUESTED_EVENT,
            Self::WorkerLaneStateChanged => WORKER_LANE_STATE_CHANGED_EVENT,
            Self::WorkerLaneWorktreeAllocated => WORKER_LANE_WORKTREE_ALLOCATED_EVENT,
            Self::WorkerLaneObservationRecorded => WORKER_LANE_OBSERVATION_RECORDED_EVENT,
        }
    }

    pub fn parse(value: &str) -> HarnessResult<Self> {
        match value {
            SESSION_STARTED_EVENT => Ok(Self::SessionStarted),
            TASK_CREATED_EVENT => Ok(Self::TaskCreated),
            TASK_ENQUEUED_EVENT => Ok(Self::TaskEnqueued),
            TASK_LEASE_ACQUIRED_EVENT => Ok(Self::TaskLeaseAcquired),
            TASK_LEASE_RENEWED_EVENT => Ok(Self::TaskLeaseRenewed),
            TASK_LEASE_EXPIRED_EVENT => Ok(Self::TaskLeaseExpired),
            TASK_LEASE_COMPLETED_EVENT => Ok(Self::TaskLeaseCompleted),
            TASK_LEASE_FAILED_EVENT => Ok(Self::TaskLeaseFailed),
            TASK_LEASE_CANCELLED_EVENT => Ok(Self::TaskLeaseCancelled),
            TASK_RETRY_QUEUED_EVENT => Ok(Self::TaskRetryQueued),
            TASK_RETRY_STOPPED_EVENT => Ok(Self::TaskRetryStopped),
            TOOL_CALL_INTENDED_EVENT => Ok(Self::ToolCallIntended),
            POLICY_DECIDED_EVENT => Ok(Self::PolicyDecided),
            TOOL_OBSERVATION_RECORDED_EVENT => Ok(Self::ToolObservationRecorded),
            CONTEXT_COMPILED_EVENT => Ok(Self::ContextCompiled),
            MODEL_REQUEST_RECORDED_EVENT => Ok(Self::ModelRequestRecorded),
            MODEL_DECISION_RECORDED_EVENT => Ok(Self::ModelDecisionRecorded),
            DIFF_RECORDED_EVENT => Ok(Self::DiffRecorded),
            COMMIT_APPROVAL_PENDING_EVENT => Ok(Self::CommitApprovalPending),
            COMMIT_APPROVED_EVENT => Ok(Self::CommitApproved),
            COMMIT_REJECTED_EVENT => Ok(Self::CommitRejected),
            COMMIT_STARTED_EVENT => Ok(Self::CommitStarted),
            COMMIT_SUCCEEDED_EVENT => Ok(Self::CommitSucceeded),
            COMMIT_FAILED_EVENT => Ok(Self::CommitFailed),
            RECOVERY_FAILURE_CLASSIFIED_EVENT => Ok(Self::RecoveryFailureClassified),
            RECOVERY_PLAN_RECORDED_EVENT => Ok(Self::RecoveryPlanRecorded),
            RECOVERY_REPAIR_ATTEMPTED_EVENT => Ok(Self::RecoveryRepairAttempted),
            RECOVERY_STOPPED_EVENT => Ok(Self::RecoveryStopped),
            WORKER_LANE_REQUESTED_EVENT => Ok(Self::WorkerLaneRequested),
            WORKER_LANE_STATE_CHANGED_EVENT => Ok(Self::WorkerLaneStateChanged),
            WORKER_LANE_WORKTREE_ALLOCATED_EVENT => Ok(Self::WorkerLaneWorktreeAllocated),
            WORKER_LANE_OBSERVATION_RECORDED_EVENT => Ok(Self::WorkerLaneObservationRecorded),
            other => Err(HarnessError::new(format!("unknown event type: {other}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStartedPayload {
    pub repo_path: String,
}

impl SessionStartedPayload {
    #[must_use]
    pub fn new(repo_path: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCreatedPayload {
    pub task_id: Uuid,
    pub repo_path: String,
    pub input: String,
    pub status: String,
    pub worker_lane_kind: String,
    pub max_context_bytes: usize,
    pub max_context_files: usize,
    pub max_context_skill_files: usize,
    pub focus_terms: Vec<String>,
    pub max_output_tokens: usize,
}

impl TaskCreatedPayload {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_id: Uuid,
        repo_path: impl Into<String>,
        input: impl Into<String>,
        status: impl Into<String>,
        worker_lane_kind: impl Into<String>,
        max_context_bytes: usize,
        max_context_files: usize,
        max_context_skill_files: usize,
        focus_terms: Vec<String>,
        max_output_tokens: usize,
    ) -> Self {
        Self {
            task_id,
            repo_path: repo_path.into(),
            input: input.into(),
            status: status.into(),
            worker_lane_kind: worker_lane_kind.into(),
            max_context_bytes,
            max_context_files,
            max_context_skill_files,
            focus_terms,
            max_output_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskQueuePayload {
    pub task_id: Uuid,
    pub status: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

impl TaskQueuePayload {
    #[must_use]
    pub fn new(task_id: Uuid, status: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            task_id,
            status: status.into(),
            reason: reason.into(),
            retry_count: None,
            max_retries: None,
            stop_reason: None,
        }
    }

    #[must_use]
    pub fn with_retry_state(
        mut self,
        retry_count: i32,
        max_retries: i32,
        stop_reason: Option<String>,
    ) -> Self {
        self.retry_count = Some(retry_count);
        self.max_retries = Some(max_retries);
        self.stop_reason = stop_reason;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLeasePayload {
    pub task_id: Uuid,
    pub lease_id: Uuid,
    pub worker_id: String,
    pub status: String,
    pub lease_deadline_ms: Option<i64>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

impl TaskLeasePayload {
    #[must_use]
    pub fn new(
        task_id: Uuid,
        lease_id: Uuid,
        worker_id: impl Into<String>,
        status: impl Into<String>,
        lease_deadline_ms: Option<i64>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            task_id,
            lease_id,
            worker_id: worker_id.into(),
            status: status.into(),
            lease_deadline_ms,
            reason: reason.into(),
            retry_count: None,
            max_retries: None,
            stop_reason: None,
        }
    }

    #[must_use]
    pub fn with_retry_state(
        mut self,
        retry_count: i32,
        max_retries: i32,
        stop_reason: Option<String>,
    ) -> Self {
        self.retry_count = Some(retry_count);
        self.max_retries = Some(max_retries);
        self.stop_reason = stop_reason;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallIntentPayload {
    pub tool_name: String,
    pub program: String,
    pub args: Vec<String>,
    pub working_dir: String,
}

impl ToolCallIntentPayload {
    #[must_use]
    pub fn new(
        tool_name: impl Into<String>,
        program: impl Into<String>,
        args: Vec<String>,
        working_dir: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            program: program.into(),
            args,
            working_dir: working_dir.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDecisionPayload {
    pub tool_name: String,
    pub decision: String,
    pub reason: String,
}

impl PolicyDecisionPayload {
    #[must_use]
    pub fn new(
        tool_name: impl Into<String>,
        decision: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            decision: decision.into(),
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolObservationPayload {
    pub tool_name: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePatchPayload {
    pub path: String,
    pub expected_content: Option<String>,
    pub replacement_content: String,
}

impl FilePatchPayload {
    #[must_use]
    pub fn new(
        path: impl Into<String>,
        expected_content: Option<String>,
        replacement_content: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            expected_content,
            replacement_content: replacement_content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePatchIntentPayload {
    pub tool_name: String,
    pub repo_path: String,
    pub patch: FilePatchPayload,
}

impl FilePatchIntentPayload {
    #[must_use]
    pub fn new(
        tool_name: impl Into<String>,
        repo_path: impl Into<String>,
        patch: FilePatchPayload,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            repo_path: repo_path.into(),
            patch,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePatchObservationPayload {
    pub tool_name: String,
    pub path: String,
    pub applied: bool,
    pub previous_bytes: usize,
    pub new_bytes: usize,
    pub duration_ms: u64,
}

impl FilePatchObservationPayload {
    #[must_use]
    pub fn new(
        tool_name: impl Into<String>,
        path: impl Into<String>,
        applied: bool,
        previous_bytes: usize,
        new_bytes: usize,
        duration_ms: u64,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            path: path.into(),
            applied,
            previous_bytes,
            new_bytes,
            duration_ms,
        }
    }
}

impl ToolObservationPayload {
    #[must_use]
    pub fn new(
        tool_name: impl Into<String>,
        exit_code: Option<i32>,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            exit_code,
            stdout: stdout.into(),
            stderr: stderr.into(),
            duration_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSourcePayload {
    pub kind: String,
    pub path: String,
    pub content: String,
    pub original_bytes: usize,
    pub included_bytes: usize,
    pub truncated: bool,
}

impl ContextSourcePayload {
    #[must_use]
    pub fn new(
        kind: impl Into<String>,
        path: impl Into<String>,
        content: impl Into<String>,
        original_bytes: usize,
        included_bytes: usize,
        truncated: bool,
    ) -> Self {
        Self {
            kind: kind.into(),
            path: path.into(),
            content: content.into(),
            original_bytes,
            included_bytes,
            truncated,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMetadataPayload {
    pub path: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

impl SkillMetadataPayload {
    #[must_use]
    pub fn new(path: impl Into<String>, name: Option<String>, description: Option<String>) -> Self {
        Self {
            path: path.into(),
            name,
            description,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextCompiledPayload {
    pub repo_path: String,
    pub budget_bytes: usize,
    pub budget_files: usize,
    pub budget_skill_files: usize,
    pub used_bytes: usize,
    pub truncated: bool,
    pub sources: Vec<ContextSourcePayload>,
    pub skills: Vec<SkillMetadataPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequestPayload {
    pub provider: String,
    pub task: String,
    pub context_source_count: usize,
    pub context_used_bytes: usize,
    pub max_output_tokens: usize,
}

impl ModelRequestPayload {
    #[must_use]
    pub fn new(
        provider: impl Into<String>,
        task: impl Into<String>,
        context_source_count: usize,
        context_used_bytes: usize,
        max_output_tokens: usize,
    ) -> Self {
        Self {
            provider: provider.into(),
            task: task.into(),
            context_source_count,
            context_used_bytes,
            max_output_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDecisionPayload {
    pub provider: String,
    pub summary: String,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub max_output_tokens: usize,
    pub patch: FilePatchPayload,
}

impl ModelDecisionPayload {
    #[must_use]
    pub fn new(
        provider: impl Into<String>,
        summary: impl Into<String>,
        prompt_tokens: usize,
        completion_tokens: usize,
        max_output_tokens: usize,
        patch: FilePatchPayload,
    ) -> Self {
        Self {
            provider: provider.into(),
            summary: summary.into(),
            prompt_tokens,
            completion_tokens,
            max_output_tokens,
            patch,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffSummaryPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub paths: Vec<String>,
    pub git_status: String,
    pub git_diff: String,
}

impl DiffSummaryPayload {
    #[must_use]
    pub fn new(
        files_changed: usize,
        insertions: usize,
        deletions: usize,
        paths: Vec<String>,
    ) -> Self {
        Self {
            task_id: None,
            files_changed,
            insertions,
            deletions,
            paths,
            git_status: String::new(),
            git_diff: String::new(),
        }
    }

    #[must_use]
    pub fn with_git_evidence(
        mut self,
        git_status: impl Into<String>,
        git_diff: impl Into<String>,
    ) -> Self {
        self.git_status = git_status.into();
        self.git_diff = git_diff.into();
        self
    }

    #[must_use]
    pub const fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    #[must_use]
    pub const fn with_optional_task_id(mut self, task_id: Option<Uuid>) -> Self {
        self.task_id = task_id;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitApprovalPendingPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    pub state: String,
    pub summary: String,
}

impl CommitApprovalPendingPayload {
    #[must_use]
    pub fn new(state: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            task_id: None,
            state: state.into(),
            summary: summary.into(),
        }
    }

    #[must_use]
    pub const fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    #[must_use]
    pub const fn with_optional_task_id(mut self, task_id: Option<Uuid>) -> Self {
        self.task_id = task_id;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitApprovalDecisionPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    pub state: String,
    pub reason: String,
    pub actor: String,
}

impl CommitApprovalDecisionPayload {
    #[must_use]
    pub fn new(
        state: impl Into<String>,
        reason: impl Into<String>,
        actor: impl Into<String>,
    ) -> Self {
        Self {
            task_id: None,
            state: state.into(),
            reason: reason.into(),
            actor: actor.into(),
        }
    }

    #[must_use]
    pub const fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitHandoffPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    pub state: String,
    pub repo_path: String,
    pub message: String,
    pub actor: String,
    pub commit_sha: Option<String>,
    pub failure_reason: Option<String>,
}

impl CommitHandoffPayload {
    #[must_use]
    pub fn started(
        repo_path: impl Into<String>,
        message: impl Into<String>,
        actor: impl Into<String>,
    ) -> Self {
        Self {
            task_id: None,
            state: "committing".to_owned(),
            repo_path: repo_path.into(),
            message: message.into(),
            actor: actor.into(),
            commit_sha: None,
            failure_reason: None,
        }
    }

    #[must_use]
    pub fn succeeded(
        repo_path: impl Into<String>,
        message: impl Into<String>,
        actor: impl Into<String>,
        commit_sha: impl Into<String>,
    ) -> Self {
        Self {
            task_id: None,
            state: "committed".to_owned(),
            repo_path: repo_path.into(),
            message: message.into(),
            actor: actor.into(),
            commit_sha: Some(commit_sha.into()),
            failure_reason: None,
        }
    }

    #[must_use]
    pub fn failed(
        repo_path: impl Into<String>,
        message: impl Into<String>,
        actor: impl Into<String>,
        failure_reason: impl Into<String>,
    ) -> Self {
        Self {
            task_id: None,
            state: "commit_failed".to_owned(),
            repo_path: repo_path.into(),
            message: message.into(),
            actor: actor.into(),
            commit_sha: None,
            failure_reason: Some(failure_reason.into()),
        }
    }

    #[must_use]
    pub const fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryFailurePayload {
    pub round: usize,
    pub classification: String,
    pub exit_code: Option<i32>,
    pub summary: String,
}

impl RecoveryFailurePayload {
    #[must_use]
    pub fn new(
        round: usize,
        classification: impl Into<String>,
        exit_code: Option<i32>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            round,
            classification: classification.into(),
            exit_code,
            summary: summary.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPlanPayload {
    pub round: usize,
    pub plan: String,
    pub max_recovery_rounds: usize,
    pub remaining_repair_bytes: usize,
}

impl RecoveryPlanPayload {
    #[must_use]
    pub fn new(
        round: usize,
        plan: impl Into<String>,
        max_recovery_rounds: usize,
        remaining_repair_bytes: usize,
    ) -> Self {
        Self {
            round,
            plan: plan.into(),
            max_recovery_rounds,
            remaining_repair_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryRepairAttemptPayload {
    pub round: usize,
    pub patch: FilePatchPayload,
    pub applied: bool,
    pub repair_bytes: usize,
}

impl RecoveryRepairAttemptPayload {
    #[must_use]
    pub fn new(round: usize, patch: FilePatchPayload, applied: bool, repair_bytes: usize) -> Self {
        Self {
            round,
            patch,
            applied,
            repair_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryStoppedPayload {
    pub stop_reason: String,
    pub retry_count: usize,
    pub final_state: String,
}

impl RecoveryStoppedPayload {
    #[must_use]
    pub fn new(
        stop_reason: impl Into<String>,
        retry_count: usize,
        final_state: impl Into<String>,
    ) -> Self {
        Self {
            stop_reason: stop_reason.into(),
            retry_count,
            final_state: final_state.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerLaneBudgetPayload {
    pub max_prompt_tokens: usize,
    pub max_output_tokens: usize,
    pub max_stdout_bytes: usize,
}

impl WorkerLaneBudgetPayload {
    #[must_use]
    pub const fn new(
        max_prompt_tokens: usize,
        max_output_tokens: usize,
        max_stdout_bytes: usize,
    ) -> Self {
        Self {
            max_prompt_tokens,
            max_output_tokens,
            max_stdout_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerLaneRequestPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    pub lane_id: String,
    pub lane_kind: String,
    pub task: String,
    pub workspace_path: String,
    pub worktree_path: Option<String>,
    pub timeout_ms: u64,
    pub cancellation_requested: bool,
    pub budget: WorkerLaneBudgetPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerLaneStatePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    pub lane_id: String,
    pub lane_kind: String,
    pub from_state: Option<String>,
    pub to_state: String,
    pub reason: String,
}

impl WorkerLaneStatePayload {
    #[must_use]
    pub fn new(
        lane_id: impl Into<String>,
        lane_kind: impl Into<String>,
        from_state: Option<String>,
        to_state: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            task_id: None,
            lane_id: lane_id.into(),
            lane_kind: lane_kind.into(),
            from_state,
            to_state: to_state.into(),
            reason: reason.into(),
        }
    }

    #[must_use]
    pub const fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    #[must_use]
    pub const fn with_optional_task_id(mut self, task_id: Option<Uuid>) -> Self {
        self.task_id = task_id;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerLaneWorktreeAllocatedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    pub lane_id: String,
    pub lane_kind: String,
    pub session_repo_path: String,
    pub worktree_path: String,
    pub base_ref: String,
}

impl WorkerLaneWorktreeAllocatedPayload {
    #[must_use]
    pub fn new(
        lane_id: impl Into<String>,
        lane_kind: impl Into<String>,
        session_repo_path: impl Into<String>,
        worktree_path: impl Into<String>,
        base_ref: impl Into<String>,
    ) -> Self {
        Self {
            task_id: None,
            lane_id: lane_id.into(),
            lane_kind: lane_kind.into(),
            session_repo_path: session_repo_path.into(),
            worktree_path: worktree_path.into(),
            base_ref: base_ref.into(),
        }
    }

    #[must_use]
    pub const fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    #[must_use]
    pub const fn with_optional_task_id(mut self, task_id: Option<Uuid>) -> Self {
        self.task_id = task_id;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerLaneObservationPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    pub lane_id: String,
    pub lane_kind: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub prompt_tokens: Option<usize>,
    pub completion_tokens: Option<usize>,
    pub usage_confidence: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub event_id: Uuid,
    pub session_id: Uuid,
    pub event_type: EventType,
    pub schema_version: u16,
    pub payload: Value,
}

impl NewEvent {
    pub fn session_started(
        session_id: Uuid,
        payload: SessionStartedPayload,
    ) -> HarnessResult<Self> {
        Ok(Self {
            event_id: Uuid::new_v4(),
            session_id,
            event_type: EventType::SessionStarted,
            schema_version: eventlog_schema_version(),
            payload: serde_json::to_value(payload)
                .map_err(|error| HarnessError::new(error.to_string()))?,
        })
    }

    pub fn task_created(session_id: Uuid, payload: TaskCreatedPayload) -> HarnessResult<Self> {
        Self::new(session_id, EventType::TaskCreated, payload)
    }

    pub fn task_enqueued(session_id: Uuid, payload: TaskQueuePayload) -> HarnessResult<Self> {
        Self::new(session_id, EventType::TaskEnqueued, payload)
    }

    pub fn task_lease_acquired(session_id: Uuid, payload: TaskLeasePayload) -> HarnessResult<Self> {
        Self::new(session_id, EventType::TaskLeaseAcquired, payload)
    }

    pub fn task_lease_renewed(session_id: Uuid, payload: TaskLeasePayload) -> HarnessResult<Self> {
        Self::new(session_id, EventType::TaskLeaseRenewed, payload)
    }

    pub fn task_lease_expired(session_id: Uuid, payload: TaskLeasePayload) -> HarnessResult<Self> {
        Self::new(session_id, EventType::TaskLeaseExpired, payload)
    }

    pub fn task_lease_completed(
        session_id: Uuid,
        payload: TaskLeasePayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::TaskLeaseCompleted, payload)
    }

    pub fn task_lease_failed(session_id: Uuid, payload: TaskLeasePayload) -> HarnessResult<Self> {
        Self::new(session_id, EventType::TaskLeaseFailed, payload)
    }

    pub fn task_lease_cancelled(
        session_id: Uuid,
        payload: TaskLeasePayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::TaskLeaseCancelled, payload)
    }

    pub fn task_retry_queued(session_id: Uuid, payload: TaskQueuePayload) -> HarnessResult<Self> {
        Self::new(session_id, EventType::TaskRetryQueued, payload)
    }

    pub fn task_retry_stopped(session_id: Uuid, payload: TaskQueuePayload) -> HarnessResult<Self> {
        Self::new(session_id, EventType::TaskRetryStopped, payload)
    }

    pub fn tool_call_intended(
        session_id: Uuid,
        payload: ToolCallIntentPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::ToolCallIntended, payload)
    }

    pub fn file_patch_intended(
        session_id: Uuid,
        payload: FilePatchIntentPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::ToolCallIntended, payload)
    }

    pub fn policy_decided(session_id: Uuid, payload: PolicyDecisionPayload) -> HarnessResult<Self> {
        Self::new(session_id, EventType::PolicyDecided, payload)
    }

    pub fn tool_observation_recorded(
        session_id: Uuid,
        payload: ToolObservationPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::ToolObservationRecorded, payload)
    }

    pub fn file_patch_observation_recorded(
        session_id: Uuid,
        payload: FilePatchObservationPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::ToolObservationRecorded, payload)
    }

    pub fn context_compiled(
        session_id: Uuid,
        payload: ContextCompiledPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::ContextCompiled, payload)
    }

    pub fn model_request_recorded(
        session_id: Uuid,
        payload: ModelRequestPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::ModelRequestRecorded, payload)
    }

    pub fn model_decision_recorded(
        session_id: Uuid,
        payload: ModelDecisionPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::ModelDecisionRecorded, payload)
    }

    pub fn diff_recorded(session_id: Uuid, payload: DiffSummaryPayload) -> HarnessResult<Self> {
        Self::new(session_id, EventType::DiffRecorded, payload)
    }

    pub fn commit_approval_pending(
        session_id: Uuid,
        payload: CommitApprovalPendingPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::CommitApprovalPending, payload)
    }

    pub fn commit_approved(
        session_id: Uuid,
        payload: CommitApprovalDecisionPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::CommitApproved, payload)
    }

    pub fn commit_rejected(
        session_id: Uuid,
        payload: CommitApprovalDecisionPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::CommitRejected, payload)
    }

    pub fn commit_started(session_id: Uuid, payload: CommitHandoffPayload) -> HarnessResult<Self> {
        Self::new(session_id, EventType::CommitStarted, payload)
    }

    pub fn commit_succeeded(
        session_id: Uuid,
        payload: CommitHandoffPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::CommitSucceeded, payload)
    }

    pub fn commit_failed(session_id: Uuid, payload: CommitHandoffPayload) -> HarnessResult<Self> {
        Self::new(session_id, EventType::CommitFailed, payload)
    }

    pub fn recovery_failure_classified(
        session_id: Uuid,
        payload: RecoveryFailurePayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::RecoveryFailureClassified, payload)
    }

    pub fn recovery_plan_recorded(
        session_id: Uuid,
        payload: RecoveryPlanPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::RecoveryPlanRecorded, payload)
    }

    pub fn recovery_repair_attempted(
        session_id: Uuid,
        payload: RecoveryRepairAttemptPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::RecoveryRepairAttempted, payload)
    }

    pub fn recovery_stopped(
        session_id: Uuid,
        payload: RecoveryStoppedPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::RecoveryStopped, payload)
    }

    pub fn worker_lane_requested(
        session_id: Uuid,
        payload: WorkerLaneRequestPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::WorkerLaneRequested, payload)
    }

    pub fn worker_lane_state_changed(
        session_id: Uuid,
        payload: WorkerLaneStatePayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::WorkerLaneStateChanged, payload)
    }

    pub fn worker_lane_worktree_allocated(
        session_id: Uuid,
        payload: WorkerLaneWorktreeAllocatedPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::WorkerLaneWorktreeAllocated, payload)
    }

    pub fn worker_lane_observation_recorded(
        session_id: Uuid,
        payload: WorkerLaneObservationPayload,
    ) -> HarnessResult<Self> {
        Self::new(
            session_id,
            EventType::WorkerLaneObservationRecorded,
            payload,
        )
    }

    fn new(
        session_id: Uuid,
        event_type: EventType,
        payload: impl Serialize,
    ) -> HarnessResult<Self> {
        Ok(Self {
            event_id: Uuid::new_v4(),
            session_id,
            event_type,
            schema_version: eventlog_schema_version(),
            payload: serde_json::to_value(payload)
                .map_err(|error| HarnessError::new(error.to_string()))?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventEnvelope {
    pub event_id: Uuid,
    pub session_id: Uuid,
    pub sequence: i64,
    pub event_type: EventType,
    pub schema_version: u16,
    pub payload: Value,
}

#[must_use]
pub const fn eventlog_schema_version() -> u16 {
    EVENTLOG_SCHEMA_VERSION
}

#[must_use]
pub fn eventlog_product_name() -> &'static str {
    harness_core::PRODUCT_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eventlog_has_initial_schema_version() {
        assert_eq!(eventlog_schema_version(), 1);
        assert_eq!(eventlog_product_name(), "Coding Agent Harness");
    }

    #[test]
    fn session_started_event_serializes_repo_path() {
        let session_id = Uuid::new_v4();
        let event = NewEvent::session_started(session_id, SessionStartedPayload::new("C:/repo"))
            .expect("session started event");

        assert_eq!(event.session_id, session_id);
        assert_eq!(event.event_type.as_str(), "session.started");
        assert_eq!(event.payload["repo_path"], "C:/repo");
    }

    #[test]
    fn task_created_event_serializes_first_class_task_metadata() {
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let event = NewEvent::task_created(
            session_id,
            TaskCreatedPayload::new(
                task_id,
                "C:/repo",
                "draft a patch",
                "created",
                "codex_cli_worker_lane",
                4096,
                8,
                2,
                vec!["agent".to_owned()],
                256,
            ),
        )
        .expect("task created event");

        assert_eq!(event.event_type.as_str(), "task.created");
        assert_eq!(event.payload["task_id"], task_id.to_string());
        assert_eq!(event.payload["repo_path"], "C:/repo");
        assert_eq!(event.payload["input"], "draft a patch");
        assert_eq!(event.payload["status"], "created");
        assert_eq!(event.payload["worker_lane_kind"], "codex_cli_worker_lane");
        assert_eq!(event.payload["max_context_bytes"], 4096);
        assert_eq!(event.payload["focus_terms"][0], "agent");
        assert_eq!(event.payload["max_output_tokens"], 256);
    }

    #[test]
    fn task_queue_and_lease_events_serialize_runtime_evidence() {
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let lease_id = Uuid::new_v4();
        let enqueued = NewEvent::task_enqueued(
            session_id,
            TaskQueuePayload::new(task_id, "queued", "task enqueued for worker execution")
                .with_retry_state(0, 1, None),
        )
        .expect("task enqueued event");
        let acquired = NewEvent::task_lease_acquired(
            session_id,
            TaskLeasePayload::new(
                task_id,
                lease_id,
                "worker-1",
                "leased",
                Some(123_456),
                "task lease acquired",
            )
            .with_retry_state(0, 1, None),
        )
        .expect("task lease acquired event");
        let renewed = NewEvent::task_lease_renewed(
            session_id,
            TaskLeasePayload::new(
                task_id,
                lease_id,
                "worker-1",
                "leased",
                Some(223_456),
                "task lease renewed",
            )
            .with_retry_state(0, 1, None),
        )
        .expect("task lease renewed event");
        let expired = NewEvent::task_lease_expired(
            session_id,
            TaskLeasePayload::new(
                task_id,
                lease_id,
                "worker-1",
                "expired",
                Some(223_456),
                "task lease expired; task released for retry",
            )
            .with_retry_state(1, 1, Some("lease_expired".to_owned())),
        )
        .expect("task lease expired event");
        let retry_queued = NewEvent::task_retry_queued(
            session_id,
            TaskQueuePayload::new(
                task_id,
                "retry_queued",
                "task lease expired; task released for retry",
            )
            .with_retry_state(1, 1, Some("lease_expired".to_owned())),
        )
        .expect("task retry queued event");
        let retry_stopped = NewEvent::task_retry_stopped(
            session_id,
            TaskQueuePayload::new(
                task_id,
                "stopped",
                "task lease expired; max retries exceeded",
            )
            .with_retry_state(1, 1, Some("max_retries_exceeded".to_owned())),
        )
        .expect("task retry stopped event");
        let completed = NewEvent::task_lease_completed(
            session_id,
            TaskLeasePayload::new(
                task_id,
                lease_id,
                "worker-1",
                "completed",
                Some(123_456),
                "worker completed task",
            ),
        )
        .expect("task lease completed event");

        assert_eq!(enqueued.event_type.as_str(), "task.enqueued");
        assert_eq!(enqueued.payload["task_id"], task_id.to_string());
        assert_eq!(enqueued.payload["status"], "queued");
        assert_eq!(enqueued.payload["retry_count"], 0);
        assert_eq!(acquired.event_type.as_str(), "task.lease_acquired");
        assert_eq!(acquired.payload["lease_id"], lease_id.to_string());
        assert_eq!(acquired.payload["worker_id"], "worker-1");
        assert_eq!(acquired.payload["lease_deadline_ms"], 123_456);
        assert_eq!(renewed.event_type.as_str(), "task.lease_renewed");
        assert_eq!(renewed.payload["lease_deadline_ms"], 223_456);
        assert_eq!(expired.event_type.as_str(), "task.lease_expired");
        assert_eq!(expired.payload["status"], "expired");
        assert_eq!(expired.payload["stop_reason"], "lease_expired");
        assert_eq!(retry_queued.event_type.as_str(), "task.retry_queued");
        assert_eq!(retry_queued.payload["status"], "retry_queued");
        assert_eq!(retry_queued.payload["retry_count"], 1);
        assert_eq!(retry_stopped.event_type.as_str(), "task.retry_stopped");
        assert_eq!(retry_stopped.payload["stop_reason"], "max_retries_exceeded");
        assert_eq!(completed.event_type.as_str(), "task.lease_completed");
        assert_eq!(completed.payload["status"], "completed");
        assert_eq!(completed.payload["reason"], "worker completed task");
    }

    #[test]
    fn tool_events_serialize_policy_and_observation_payloads() {
        let session_id = Uuid::new_v4();
        let intent = NewEvent::tool_call_intended(
            session_id,
            ToolCallIntentPayload::new(
                "verify_command",
                "cargo",
                vec!["test".to_owned()],
                "C:/repo",
            ),
        )
        .expect("tool intent event");
        let decision = NewEvent::policy_decided(
            session_id,
            PolicyDecisionPayload::new("verify_command", "allow", "safe"),
        )
        .expect("policy decision event");
        let observation = NewEvent::tool_observation_recorded(
            session_id,
            ToolObservationPayload::new("verify_command", Some(0), "ok", "", 10),
        )
        .expect("tool observation event");

        assert_eq!(intent.event_type.as_str(), "tool.call_intended");
        assert_eq!(intent.payload["program"], "cargo");
        assert_eq!(decision.payload["decision"], "allow");
        assert_eq!(observation.payload["exit_code"], 0);
    }

    #[test]
    fn context_compiled_event_serializes_bounded_bundle() {
        let session_id = Uuid::new_v4();
        let event = NewEvent::context_compiled(
            session_id,
            ContextCompiledPayload {
                repo_path: "C:/repo".to_owned(),
                budget_bytes: 4096,
                budget_files: 8,
                budget_skill_files: 4,
                used_bytes: 12,
                truncated: false,
                sources: vec![ContextSourcePayload::new(
                    "repository_instructions",
                    "AGENTS.md",
                    "instructions",
                    12,
                    12,
                    false,
                )],
                skills: vec![SkillMetadataPayload::new(
                    ".codex/skills/demo/SKILL.md",
                    Some("demo".to_owned()),
                    Some("Demo skill".to_owned()),
                )],
            },
        )
        .expect("context compiled event");

        assert_eq!(event.event_type.as_str(), "context.compiled");
        assert_eq!(event.payload["budget_bytes"], 4096);
        assert_eq!(event.payload["budget_files"], 8);
        assert_eq!(event.payload["budget_skill_files"], 4);
        assert_eq!(event.payload["sources"][0]["path"], "AGENTS.md");
        assert_eq!(event.payload["skills"][0]["name"], "demo");
    }

    #[test]
    fn model_events_serialize_request_decision_and_patch() {
        let session_id = Uuid::new_v4();
        let patch = FilePatchPayload::new(".harness/fake-agent-turn.md", None, "fake patch");
        let request = NewEvent::model_request_recorded(
            session_id,
            ModelRequestPayload::new("deterministic-fake-model", "write fixture", 1, 128, 256),
        )
        .expect("model request event");
        let decision = NewEvent::model_decision_recorded(
            session_id,
            ModelDecisionPayload::new(
                "deterministic-fake-model",
                "propose patch",
                40,
                12,
                256,
                patch.clone(),
            ),
        )
        .expect("model decision event");
        let intent = NewEvent::file_patch_intended(
            session_id,
            FilePatchIntentPayload::new("apply_file_patch", "C:/repo", patch),
        )
        .expect("file patch intent event");

        assert_eq!(request.event_type.as_str(), "model.request_recorded");
        assert_eq!(request.payload["context_used_bytes"], 128);
        assert_eq!(
            decision.payload["patch"]["path"],
            ".harness/fake-agent-turn.md"
        );
        assert_eq!(intent.event_type.as_str(), "tool.call_intended");
        assert_eq!(intent.payload["tool_name"], "apply_file_patch");
    }

    #[test]
    fn coding_task_events_serialize_diff_and_pending_approval() {
        let session_id = Uuid::new_v4();
        let diff = NewEvent::diff_recorded(
            session_id,
            DiffSummaryPayload::new(1, 4, 0, vec![".harness/fake-agent-turn.md".to_owned()]),
        )
        .expect("diff event");
        let pending = NewEvent::commit_approval_pending(
            session_id,
            CommitApprovalPendingPayload::new(
                "pending_commit_approval",
                "verification passed; awaiting human commit approval",
            ),
        )
        .expect("commit approval pending event");
        let approved = NewEvent::commit_approved(
            session_id,
            CommitApprovalDecisionPayload::new("approved", "approved by runtime", "runtime"),
        )
        .expect("commit approved event");
        let rejected = NewEvent::commit_rejected(
            session_id,
            CommitApprovalDecisionPayload::new("rejected", "not the intended change", "runtime"),
        )
        .expect("commit rejected event");
        let started = NewEvent::commit_started(
            session_id,
            CommitHandoffPayload::started("C:/repo", "Commit approved patch", "runtime"),
        )
        .expect("commit started event");
        let succeeded = NewEvent::commit_succeeded(
            session_id,
            CommitHandoffPayload::succeeded(
                "C:/repo",
                "Commit approved patch",
                "runtime",
                "0123456789abcdef0123456789abcdef01234567",
            ),
        )
        .expect("commit succeeded event");
        let failed = NewEvent::commit_failed(
            session_id,
            CommitHandoffPayload::failed(
                "C:/repo",
                "Commit approved patch",
                "runtime",
                "git failed",
            ),
        )
        .expect("commit failed event");

        assert_eq!(diff.event_type.as_str(), "diff.recorded");
        assert!(diff.payload.get("task_id").is_none());
        assert_eq!(diff.payload["files_changed"], 1);
        assert_eq!(diff.payload["paths"][0], ".harness/fake-agent-turn.md");
        assert_eq!(pending.event_type.as_str(), "commit.approval_pending");
        assert_eq!(pending.payload["state"], "pending_commit_approval");
        assert_eq!(approved.event_type.as_str(), "commit.approved");
        assert_eq!(approved.payload["state"], "approved");
        assert_eq!(approved.payload["actor"], "runtime");
        assert_eq!(rejected.event_type.as_str(), "commit.rejected");
        assert_eq!(rejected.payload["state"], "rejected");
        assert_eq!(rejected.payload["reason"], "not the intended change");
        assert_eq!(started.event_type.as_str(), "commit.started");
        assert_eq!(started.payload["state"], "committing");
        assert_eq!(started.payload["repo_path"], "C:/repo");
        assert_eq!(started.payload["commit_sha"], serde_json::Value::Null);
        assert_eq!(succeeded.event_type.as_str(), "commit.succeeded");
        assert_eq!(succeeded.payload["state"], "committed");
        assert_eq!(
            succeeded.payload["commit_sha"],
            "0123456789abcdef0123456789abcdef01234567"
        );
        assert_eq!(failed.event_type.as_str(), "commit.failed");
        assert_eq!(failed.payload["state"], "commit_failed");
        assert_eq!(failed.payload["failure_reason"], "git failed");
    }

    #[test]
    fn recovery_events_serialize_loop_report() {
        let session_id = Uuid::new_v4();
        let patch = FilePatchPayload::new(
            ".harness/fake-agent-turn.md",
            Some("before".to_owned()),
            "before\nrecovered=true\n",
        );
        let failure = NewEvent::recovery_failure_classified(
            session_id,
            RecoveryFailurePayload::new(
                0,
                "fixture_missing_recovery_marker",
                Some(101),
                "first verification failed",
            ),
        )
        .expect("failure event");
        let plan = NewEvent::recovery_plan_recorded(
            session_id,
            RecoveryPlanPayload::new(1, "append recovered=true", 2, 4096),
        )
        .expect("plan event");
        let attempt = NewEvent::recovery_repair_attempted(
            session_id,
            RecoveryRepairAttemptPayload::new(1, patch, true, 22),
        )
        .expect("attempt event");
        let stopped = NewEvent::recovery_stopped(
            session_id,
            RecoveryStoppedPayload::new("recovered", 1, "pending_commit_approval"),
        )
        .expect("stopped event");

        assert_eq!(failure.event_type.as_str(), "recovery.failure_classified");
        assert_eq!(
            failure.payload["classification"],
            "fixture_missing_recovery_marker"
        );
        assert_eq!(plan.payload["max_recovery_rounds"], 2);
        assert_eq!(attempt.payload["applied"], true);
        assert_eq!(stopped.payload["retry_count"], 1);
    }

    #[test]
    fn worker_lane_events_serialize_contract_state_and_observation() {
        let session_id = Uuid::new_v4();
        let requested = NewEvent::worker_lane_requested(
            session_id,
            WorkerLaneRequestPayload {
                task_id: None,
                lane_id: "lane-1".to_owned(),
                lane_kind: "codex_cli".to_owned(),
                task: "draft a patch".to_owned(),
                workspace_path: "C:/repo".to_owned(),
                worktree_path: None,
                timeout_ms: 30_000,
                cancellation_requested: false,
                budget: WorkerLaneBudgetPayload::new(8_192, 2_048, 64 * 1024),
            },
        )
        .expect("worker lane request event");
        let running = NewEvent::worker_lane_state_changed(
            session_id,
            WorkerLaneStatePayload::new(
                "lane-1",
                "codex_cli",
                Some("queued".to_owned()),
                "running",
                "worker accepted by policy",
            ),
        )
        .expect("worker lane state event");
        let allocated = NewEvent::worker_lane_worktree_allocated(
            session_id,
            WorkerLaneWorktreeAllocatedPayload::new(
                "lane-1",
                "codex_cli",
                "C:/repo",
                "C:/repo-worktree",
                "HEAD",
            ),
        )
        .expect("worker lane worktree event");
        let observation = NewEvent::worker_lane_observation_recorded(
            session_id,
            WorkerLaneObservationPayload {
                task_id: None,
                lane_id: "lane-1".to_owned(),
                lane_kind: "codex_cli".to_owned(),
                status: "succeeded".to_owned(),
                exit_code: Some(0),
                stdout: "codex proposed a patch".to_owned(),
                stderr: String::new(),
                duration_ms: 42,
                prompt_tokens: Some(120),
                completion_tokens: Some(40),
                usage_confidence: "local_estimate".to_owned(),
            },
        )
        .expect("worker lane observation event");

        assert_eq!(requested.event_type.as_str(), "worker_lane.requested");
        assert!(requested.payload.get("task_id").is_none());
        assert_eq!(requested.payload["task"], "draft a patch");
        assert_eq!(requested.payload["budget"]["max_output_tokens"], 2_048);
        assert_eq!(running.event_type.as_str(), "worker_lane.state_changed");
        assert_eq!(running.payload["from_state"], "queued");
        assert_eq!(running.payload["to_state"], "running");
        assert_eq!(
            allocated.event_type.as_str(),
            "worker_lane.worktree_allocated"
        );
        assert_eq!(allocated.payload["worktree_path"], "C:/repo-worktree");
        assert_eq!(allocated.payload["base_ref"], "HEAD");
        assert_eq!(
            observation.event_type.as_str(),
            "worker_lane.observation_recorded"
        );
        assert_eq!(observation.payload["usage_confidence"], "local_estimate");
        assert_eq!(observation.payload["stdout"], "codex proposed a patch");
    }
}
