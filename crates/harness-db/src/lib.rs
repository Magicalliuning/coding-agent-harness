use std::sync::Mutex;

use harness_core::{HarnessError, HarnessResult};
use harness_events::{EventEnvelope, EventType, NewEvent};
use postgres::{Client, NoTls};
use uuid::Uuid;

pub const DATABASE_URL_ENV: &str = "HARNESS_DATABASE_URL";

pub const MIGRATIONS_DIR: &str = "migrations";

const RUNTIME_BASELINE_MIGRATION: &str =
    include_str!("../../../migrations/0001_runtime_baseline.sql");
const TASK_QUEUE_MIGRATION: &str = include_str!("../../../migrations/0002_task_queue.sql");

pub struct PostgresEventStore {
    client: Mutex<Client>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskQueueRecord {
    pub session_id: Uuid,
    pub task_id: Uuid,
    pub status: String,
    pub worker_id: Option<String>,
    pub lease_id: Option<Uuid>,
    pub lease_deadline_ms: Option<i64>,
    pub last_reason: Option<String>,
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
            .batch_execute(&format!(
                "{RUNTIME_BASELINE_MIGRATION}\n{TASK_QUEUE_MIGRATION}"
            ))
            .map_err(|error| HarnessError::new(error.to_string()))
    }

    pub fn append_event(&self, event: NewEvent) -> HarnessResult<EventEnvelope> {
        let mut client = self.client()?;
        let session_lock_key = event.session_id.to_string();
        let mut transaction = client
            .transaction()
            .map_err(|error| HarnessError::new(error.to_string()))?;

        transaction
            .execute(
                "SELECT pg_advisory_xact_lock(hashtext($1::text)::bigint)",
                &[&session_lock_key],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        let row = transaction
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

        transaction
            .commit()
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

    pub fn enqueue_task(&self, session_id: Uuid, task_id: Uuid) -> HarnessResult<TaskQueueRecord> {
        let mut client = self.client()?;
        let row = client
            .query_one(
                "
                INSERT INTO harness_runtime.task_queue (
                    task_id,
                    session_id,
                    status,
                    last_reason
                )
                VALUES ($1, $2, 'queued', 'task enqueued for worker execution')
                RETURNING session_id, task_id, status, worker_id, lease_id, lease_deadline_ms, last_reason
                ",
                &[&task_id, &session_id],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(task_queue_record_from_row(row))
    }

    pub fn lease_next_queued_task(
        &self,
        worker_id: &str,
        lease_id: Uuid,
        lease_deadline_ms: i64,
    ) -> HarnessResult<Option<TaskQueueRecord>> {
        let mut client = self.client()?;
        let mut transaction = client
            .transaction()
            .map_err(|error| HarnessError::new(error.to_string()))?;
        let row = transaction
            .query_opt(
                "
                WITH candidate AS (
                    SELECT task_id
                    FROM harness_runtime.task_queue
                    WHERE status = 'queued'
                    ORDER BY created_at ASC
                    FOR UPDATE SKIP LOCKED
                    LIMIT 1
                )
                UPDATE harness_runtime.task_queue task_queue
                SET
                    status = 'leased',
                    worker_id = $1,
                    lease_id = $2,
                    lease_deadline_ms = $3,
                    last_reason = 'task lease acquired',
                    updated_at = now()
                FROM candidate
                WHERE task_queue.task_id = candidate.task_id
                RETURNING task_queue.session_id, task_queue.task_id, task_queue.status,
                    task_queue.worker_id, task_queue.lease_id, task_queue.lease_deadline_ms,
                    task_queue.last_reason
                ",
                &[&worker_id, &lease_id, &lease_deadline_ms],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        transaction
            .commit()
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(row.map(task_queue_record_from_row))
    }

    pub fn transition_leased_task(
        &self,
        task_id: Uuid,
        lease_id: Uuid,
        status: &str,
        reason: &str,
    ) -> HarnessResult<TaskQueueRecord> {
        let mut client = self.client()?;
        let row = client
            .query_opt(
                "
                UPDATE harness_runtime.task_queue
                SET
                    status = $3,
                    last_reason = $4,
                    updated_at = now()
                WHERE task_id = $1
                    AND lease_id = $2
                    AND status = 'leased'
                RETURNING session_id, task_id, status, worker_id, lease_id, lease_deadline_ms, last_reason
                ",
                &[&task_id, &lease_id, &status, &reason],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        row.map(task_queue_record_from_row)
            .ok_or_else(|| HarnessError::new("active task lease was not found"))
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

fn task_queue_record_from_row(row: postgres::Row) -> TaskQueueRecord {
    TaskQueueRecord {
        session_id: row.get("session_id"),
        task_id: row.get("task_id"),
        status: row.get("status"),
        worker_id: row.get("worker_id"),
        lease_id: row.get("lease_id"),
        lease_deadline_ms: row.get("lease_deadline_ms"),
        last_reason: row.get("last_reason"),
    }
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
        assert!(TASK_QUEUE_MIGRATION.contains("harness_runtime.task_queue"));
    }
}
