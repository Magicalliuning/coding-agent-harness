use harness_core::{HarnessError, HarnessResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const EVENTLOG_SCHEMA_VERSION: u16 = 1;

pub const SESSION_STARTED_EVENT: &str = "session.started";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    SessionStarted,
}

impl EventType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStarted => SESSION_STARTED_EVENT,
        }
    }

    pub fn parse(value: &str) -> HarnessResult<Self> {
        match value {
            SESSION_STARTED_EVENT => Ok(Self::SessionStarted),
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
}
