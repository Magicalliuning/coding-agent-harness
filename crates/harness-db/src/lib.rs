use std::sync::Mutex;

use harness_core::{HarnessError, HarnessResult};
use harness_events::{EventEnvelope, EventType, NewEvent};
use postgres::{Client, NoTls, Transaction};
use uuid::Uuid;

pub const DATABASE_URL_ENV: &str = "HARNESS_DATABASE_URL";

pub const MIGRATIONS_DIR: &str = "migrations";

const RUNTIME_BASELINE_MIGRATION: &str =
    include_str!("../../../migrations/0001_runtime_baseline.sql");
const TASK_QUEUE_MIGRATION: &str = include_str!("../../../migrations/0002_task_queue.sql");
const TASK_LEASE_RECOVERY_MIGRATION: &str =
    include_str!("../../../migrations/0003_task_lease_recovery.sql");

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
    pub retry_count: i32,
    pub max_retries: i32,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskLeaseExpirationRecord {
    pub queue: TaskQueueRecord,
    pub expired_lease_id: Uuid,
    pub expired_worker_id: String,
    pub expired_deadline_ms: i64,
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
                "{RUNTIME_BASELINE_MIGRATION}\n{TASK_QUEUE_MIGRATION}\n{TASK_LEASE_RECOVERY_MIGRATION}"
            ))
            .map_err(|error| HarnessError::new(error.to_string()))
    }

    pub fn append_event(&self, event: NewEvent) -> HarnessResult<EventEnvelope> {
        let mut client = self.client()?;
        let mut transaction = client
            .transaction()
            .map_err(|error| HarnessError::new(error.to_string()))?;

        let event = append_event_in_transaction(&mut transaction, event)?;

        transaction
            .commit()
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(event)
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

    pub fn task_queue_record(&self, task_id: Uuid) -> HarnessResult<Option<TaskQueueRecord>> {
        let mut client = self.client()?;
        let row = client
            .query_opt(
                "
                SELECT session_id, task_id, status, worker_id, lease_id, lease_deadline_ms,
                    last_reason, retry_count, max_retries, stop_reason
                FROM harness_runtime.task_queue
                WHERE task_id = $1
                ",
                &[&task_id],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(row.map(task_queue_record_from_row))
    }

    pub fn task_queue_records(
        &self,
        session_id: Option<Uuid>,
    ) -> HarnessResult<Vec<TaskQueueRecord>> {
        let mut client = self.client()?;
        let rows = client
            .query(
                "
                SELECT session_id, task_id, status, worker_id, lease_id, lease_deadline_ms,
                    last_reason, retry_count, max_retries, stop_reason
                FROM harness_runtime.task_queue
                WHERE ($1::uuid IS NULL OR session_id = $1)
                ORDER BY created_at ASC, task_id ASC
                ",
                &[&session_id],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(rows.into_iter().map(task_queue_record_from_row).collect())
    }

    pub fn enqueue_task(
        &self,
        session_id: Uuid,
        task_id: Uuid,
        max_retries: i32,
        make_event: impl FnOnce(&TaskQueueRecord) -> HarnessResult<NewEvent>,
    ) -> HarnessResult<TaskQueueRecord> {
        let mut client = self.client()?;
        let mut transaction = client
            .transaction()
            .map_err(|error| HarnessError::new(error.to_string()))?;
        let row = transaction
            .query_one(
                "
                INSERT INTO harness_runtime.task_queue (
                    task_id,
                    session_id,
                    status,
                    last_reason,
                    max_retries
                )
                VALUES ($1, $2, 'queued', 'task enqueued for worker execution', $3)
                RETURNING session_id, task_id, status, worker_id, lease_id, lease_deadline_ms,
                    last_reason, retry_count, max_retries, stop_reason
                ",
                &[&task_id, &session_id, &max_retries],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        let record = task_queue_record_from_row(row);
        append_event_in_transaction(&mut transaction, make_event(&record)?)?;

        transaction
            .commit()
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(record)
    }

    pub fn lease_next_queued_task(
        &self,
        worker_id: &str,
        lease_id: Uuid,
        lease_deadline_ms: i64,
        make_event: impl FnOnce(&TaskQueueRecord) -> HarnessResult<NewEvent>,
    ) -> HarnessResult<Option<TaskQueueRecord>> {
        self.lease_next_queued_task_matching(
            worker_id,
            lease_id,
            lease_deadline_ms,
            None,
            make_event,
        )
    }

    pub fn lease_next_queued_task_for_worker_lane(
        &self,
        worker_id: &str,
        lease_id: Uuid,
        lease_deadline_ms: i64,
        worker_lane_kind: &str,
        make_event: impl FnOnce(&TaskQueueRecord) -> HarnessResult<NewEvent>,
    ) -> HarnessResult<Option<TaskQueueRecord>> {
        self.lease_next_queued_task_matching(
            worker_id,
            lease_id,
            lease_deadline_ms,
            Some(worker_lane_kind),
            make_event,
        )
    }

    fn lease_next_queued_task_matching(
        &self,
        worker_id: &str,
        lease_id: Uuid,
        lease_deadline_ms: i64,
        worker_lane_kind: Option<&str>,
        make_event: impl FnOnce(&TaskQueueRecord) -> HarnessResult<NewEvent>,
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
                    WHERE status IN ('queued', 'retry_queued')
                        AND (
                            $4::text IS NULL
                            OR EXISTS (
                                SELECT 1
                                FROM harness_runtime.event_log
                                WHERE event_log.session_id = task_queue.session_id
                                    AND event_log.event_type = 'task.created'
                                    AND event_log.payload->>'task_id' = task_queue.task_id::text
                                    AND event_log.payload->>'worker_lane_kind' = $4
                            )
                        )
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
                    stop_reason = NULL,
                    last_reason = 'task lease acquired',
                    updated_at = now()
                FROM candidate
                WHERE task_queue.task_id = candidate.task_id
                RETURNING task_queue.session_id, task_queue.task_id, task_queue.status,
                    task_queue.worker_id, task_queue.lease_id, task_queue.lease_deadline_ms,
                    task_queue.last_reason, task_queue.retry_count, task_queue.max_retries,
                    task_queue.stop_reason
                ",
                &[&worker_id, &lease_id, &lease_deadline_ms, &worker_lane_kind],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        let record = row.map(task_queue_record_from_row);
        if let Some(record) = &record {
            append_event_in_transaction(&mut transaction, make_event(record)?)?;
        }

        transaction
            .commit()
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(record)
    }

    pub fn renew_task_lease(
        &self,
        task_id: Uuid,
        lease_id: Uuid,
        worker_id: &str,
        now_ms: i64,
        lease_deadline_ms: i64,
        make_event: impl FnOnce(&TaskQueueRecord) -> HarnessResult<NewEvent>,
    ) -> HarnessResult<TaskQueueRecord> {
        let mut client = self.client()?;
        let mut transaction = client
            .transaction()
            .map_err(|error| HarnessError::new(error.to_string()))?;
        let row = transaction
            .query_opt(
                "
                UPDATE harness_runtime.task_queue
                SET
                    lease_deadline_ms = $5,
                    last_reason = 'task lease renewed',
                    updated_at = now()
                WHERE task_id = $1
                    AND lease_id = $2
                    AND worker_id = $3
                    AND status = 'leased'
                    AND lease_deadline_ms > $4
                RETURNING session_id, task_id, status, worker_id, lease_id, lease_deadline_ms,
                    last_reason, retry_count, max_retries, stop_reason
                ",
                &[&task_id, &lease_id, &worker_id, &now_ms, &lease_deadline_ms],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        let record = row
            .map(task_queue_record_from_row)
            .ok_or_else(|| HarnessError::new("active task lease was not found"))?;
        append_event_in_transaction(&mut transaction, make_event(&record)?)?;

        transaction
            .commit()
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(record)
    }

    pub fn expire_due_task_leases(
        &self,
        now_ms: i64,
        mut make_events: impl FnMut(&TaskLeaseExpirationRecord) -> HarnessResult<Vec<NewEvent>>,
    ) -> HarnessResult<Vec<TaskLeaseExpirationRecord>> {
        let mut client = self.client()?;
        let mut transaction = client
            .transaction()
            .map_err(|error| HarnessError::new(error.to_string()))?;
        let due_rows = transaction
            .query(
                "
                SELECT session_id, task_id, worker_id, lease_id, lease_deadline_ms,
                    retry_count, max_retries
                FROM harness_runtime.task_queue
                WHERE status = 'leased'
                    AND lease_deadline_ms <= $1
                ORDER BY updated_at ASC
                FOR UPDATE SKIP LOCKED
                ",
                &[&now_ms],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        let mut expired = Vec::new();

        for row in due_rows {
            let task_id: Uuid = row.get("task_id");
            let expired_lease_id: Uuid = row
                .get::<_, Option<Uuid>>("lease_id")
                .ok_or_else(|| HarnessError::new("expired task lease id was not recorded"))?;
            let expired_worker_id: String = row
                .get::<_, Option<String>>("worker_id")
                .ok_or_else(|| HarnessError::new("expired task worker id was not recorded"))?;
            let expired_deadline_ms: i64 = row
                .get::<_, Option<i64>>("lease_deadline_ms")
                .ok_or_else(|| HarnessError::new("expired task deadline was not recorded"))?;
            let retry_count: i32 = row.get("retry_count");
            let max_retries: i32 = row.get("max_retries");
            let (next_status, next_retry_count, stop_reason, reason) = if retry_count < max_retries
            {
                (
                    "retry_queued",
                    retry_count + 1,
                    Some("lease_expired"),
                    "task lease expired; task released for retry",
                )
            } else {
                (
                    "stopped",
                    retry_count,
                    Some("max_retries_exceeded"),
                    "task lease expired; max retries exceeded",
                )
            };

            let queue_row = transaction
                .query_one(
                    "
                    UPDATE harness_runtime.task_queue
                    SET
                        status = $2,
                        worker_id = CASE WHEN $2 = 'retry_queued' THEN NULL ELSE worker_id END,
                        lease_id = CASE WHEN $2 = 'retry_queued' THEN NULL ELSE lease_id END,
                        lease_deadline_ms = CASE WHEN $2 = 'retry_queued' THEN NULL ELSE lease_deadline_ms END,
                        retry_count = $3,
                        stop_reason = $4,
                        last_reason = $5,
                        updated_at = now()
                    WHERE task_id = $1
                    RETURNING session_id, task_id, status, worker_id, lease_id, lease_deadline_ms,
                        last_reason, retry_count, max_retries, stop_reason
                    ",
                    &[&task_id, &next_status, &next_retry_count, &stop_reason, &reason],
                )
                .map_err(|error| HarnessError::new(error.to_string()))?;

            expired.push(TaskLeaseExpirationRecord {
                queue: task_queue_record_from_row(queue_row),
                expired_lease_id,
                expired_worker_id,
                expired_deadline_ms,
            });
            let record = expired
                .last()
                .ok_or_else(|| HarnessError::new("expired task record was not recorded"))?;
            for event in make_events(record)? {
                append_event_in_transaction(&mut transaction, event)?;
            }
        }

        transaction
            .commit()
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(expired)
    }

    pub fn transition_leased_task(
        &self,
        task_id: Uuid,
        lease_id: Uuid,
        status: &str,
        reason: &str,
        now_ms: i64,
        make_event: impl FnOnce(&TaskQueueRecord) -> HarnessResult<NewEvent>,
    ) -> HarnessResult<TaskQueueRecord> {
        let mut client = self.client()?;
        let mut transaction = client
            .transaction()
            .map_err(|error| HarnessError::new(error.to_string()))?;
        let row = transaction
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
                    AND lease_deadline_ms > $5
                RETURNING session_id, task_id, status, worker_id, lease_id, lease_deadline_ms,
                    last_reason, retry_count, max_retries, stop_reason
                ",
                &[&task_id, &lease_id, &status, &reason, &now_ms],
            )
            .map_err(|error| HarnessError::new(error.to_string()))?;

        let record = if let Some(row) = row {
            task_queue_record_from_row(row)
        } else {
            let expired = transaction
                .query_opt(
                    "
                    SELECT 1
                    FROM harness_runtime.task_queue
                    WHERE task_id = $1
                        AND lease_id = $2
                        AND status = 'leased'
                        AND lease_deadline_ms <= $3
                    ",
                    &[&task_id, &lease_id, &now_ms],
                )
                .map_err(|error| HarnessError::new(error.to_string()))?;

            if expired.is_some() {
                return Err(HarnessError::new("task lease expired"));
            }

            return Err(HarnessError::new("active task lease was not found"));
        };
        append_event_in_transaction(&mut transaction, make_event(&record)?)?;

        transaction
            .commit()
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(record)
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

fn append_event_in_transaction(
    transaction: &mut Transaction<'_>,
    event: NewEvent,
) -> HarnessResult<EventEnvelope> {
    let session_lock_key = event.session_id.to_string();
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

    event_from_row(row)
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
        retry_count: row.get("retry_count"),
        max_retries: row.get("max_retries"),
        stop_reason: row.get("stop_reason"),
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
        assert!(TASK_LEASE_RECOVERY_MIGRATION.contains("retry_count"));
    }
}
