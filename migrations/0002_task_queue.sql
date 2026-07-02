CREATE TABLE IF NOT EXISTS harness_runtime.task_queue (
    task_id UUID PRIMARY KEY,
    session_id UUID NOT NULL,
    status TEXT NOT NULL,
    worker_id TEXT,
    lease_id UUID,
    lease_deadline_ms BIGINT,
    last_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS task_queue_status_created_idx
    ON harness_runtime.task_queue (status, created_at);
