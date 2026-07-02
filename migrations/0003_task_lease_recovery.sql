ALTER TABLE harness_runtime.task_queue
    ADD COLUMN IF NOT EXISTS retry_count INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS max_retries INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS stop_reason TEXT;

CREATE INDEX IF NOT EXISTS task_queue_leased_deadline_idx
    ON harness_runtime.task_queue (lease_deadline_ms)
    WHERE status = 'leased';
