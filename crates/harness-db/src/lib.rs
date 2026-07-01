use std::sync::Mutex;

use harness_core::{HarnessError, HarnessResult};
use harness_events::{EventEnvelope, EventType, NewEvent};
use postgres::{Client, NoTls};
use uuid::Uuid;

pub const DATABASE_URL_ENV: &str = "HARNESS_DATABASE_URL";

pub const MIGRATIONS_DIR: &str = "migrations";

const RUNTIME_BASELINE_MIGRATION: &str =
    include_str!("../../../migrations/0001_runtime_baseline.sql");

pub struct PostgresEventStore {
    client: Mutex<Client>,
}

impl PostgresEventStore {
    pub fn connect(database_url: &str) -> HarnessResult<Self> {
        let client = Client::connect(database_url, NoTls)
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(Self {
            client: Mutex::new(client),
        })
    }

    pub fn apply_migrations(&self) -> HarnessResult<()> {
        let mut client = self.client()?;
        client
            .batch_execute(RUNTIME_BASELINE_MIGRATION)
            .map_err(|error| HarnessError::new(error.to_string()))
    }

    pub fn append_event(&self, event: NewEvent) -> HarnessResult<EventEnvelope> {
        let mut client = self.client()?;
        let row = client
            .query_one(
                "
                INSERT INTO harness_runtime.event_log (
                    event_id,
                    session_id,
                    sequence,
                    event_type,
                    schema_version,
                    payload
                )
                VALUES (
                    $1,
                    $2,
                    (
                        SELECT COALESCE(MAX(sequence), 0) + 1
                        FROM harness_runtime.event_log
                        WHERE session_id = $2
                    ),
                    $3,
                    $4,
                    $5
                )
                RETURNING event_id, session_id, sequence, event_type, schema_version, payload
                ",
                &[
                    &event.event_id,
                    &event.session_id,
                    &event.event_type.as_str(),
                    &i32::from(event.schema_version),
                    &event.payload,
                ],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        event_from_row(row)
    }

    pub fn events_for_session(&self, session_id: Uuid) -> HarnessResult<Vec<EventEnvelope>> {
        let mut client = self.client()?;
        let rows = client
            .query(
                "
                SELECT event_id, session_id, sequence, event_type, schema_version, payload
                FROM harness_runtime.event_log
                WHERE session_id = $1
                ORDER BY sequence ASC
                ",
                &[&session_id],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        rows.into_iter().map(event_from_row).collect()
    }

    fn client(&self) -> HarnessResult<std::sync::MutexGuard<'_, Client>> {
        self.client
            .lock()
            .map_err(|_| HarnessError::new("database client lock poisoned"))
    }
}

#[must_use]
pub fn baseline_summary() -> String {
    format!(
        "{} uses EventLog schema v{} with migrations in {MIGRATIONS_DIR}",
        harness_core::PRODUCT_NAME,
        harness_events::eventlog_schema_version()
    )
}

fn event_from_row(row: postgres::Row) -> HarnessResult<EventEnvelope> {
    let event_type: String = row.get("event_type");
    let schema_version: i32 = row.get("schema_version");

    let schema_version = u16::try_from(schema_version)
        .map_err(|_| HarnessError::new("event schema version is out of range"))?;

    Ok(EventEnvelope {
        event_id: row.get("event_id"),
        session_id: row.get("session_id"),
        sequence: row.get("sequence"),
        event_type: EventType::parse(&event_type)?,
        schema_version,
        payload: row.get("payload"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_names_database_env_and_migration_path() {
        assert_eq!(DATABASE_URL_ENV, "HARNESS_DATABASE_URL");
        assert_eq!(MIGRATIONS_DIR, "migrations");
        assert!(baseline_summary().contains("EventLog schema v1"));
    }

    #[test]
    fn baseline_migration_creates_event_log_table() {
        assert!(RUNTIME_BASELINE_MIGRATION.contains("harness_runtime.event_log"));
    }
}
