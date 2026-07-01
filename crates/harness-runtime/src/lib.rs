use harness_core::{HarnessError, HarnessResult};
use harness_db::PostgresEventStore;
use harness_events::{EventEnvelope, EventType, NewEvent, SessionStartedPayload};
use uuid::Uuid;

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
}
