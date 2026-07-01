use harness_core::{HarnessError, HarnessResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const EVENTLOG_SCHEMA_VERSION: u16 = 1;

pub const SESSION_STARTED_EVENT: &str = "session.started";
pub const TOOL_CALL_INTENDED_EVENT: &str = "tool.call_intended";
pub const POLICY_DECIDED_EVENT: &str = "policy.decided";
pub const TOOL_OBSERVATION_RECORDED_EVENT: &str = "tool.observation_recorded";
pub const CONTEXT_COMPILED_EVENT: &str = "context.compiled";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    SessionStarted,
    ToolCallIntended,
    PolicyDecided,
    ToolObservationRecorded,
    ContextCompiled,
}

impl EventType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStarted => SESSION_STARTED_EVENT,
            Self::ToolCallIntended => TOOL_CALL_INTENDED_EVENT,
            Self::PolicyDecided => POLICY_DECIDED_EVENT,
            Self::ToolObservationRecorded => TOOL_OBSERVATION_RECORDED_EVENT,
            Self::ContextCompiled => CONTEXT_COMPILED_EVENT,
        }
    }

    pub fn parse(value: &str) -> HarnessResult<Self> {
        match value {
            SESSION_STARTED_EVENT => Ok(Self::SessionStarted),
            TOOL_CALL_INTENDED_EVENT => Ok(Self::ToolCallIntended),
            POLICY_DECIDED_EVENT => Ok(Self::PolicyDecided),
            TOOL_OBSERVATION_RECORDED_EVENT => Ok(Self::ToolObservationRecorded),
            CONTEXT_COMPILED_EVENT => Ok(Self::ContextCompiled),
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

    pub fn tool_call_intended(
        session_id: Uuid,
        payload: ToolCallIntentPayload,
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

    pub fn context_compiled(
        session_id: Uuid,
        payload: ContextCompiledPayload,
    ) -> HarnessResult<Self> {
        Self::new(session_id, EventType::ContextCompiled, payload)
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
}
