CREATE SCHEMA IF NOT EXISTS harness_runtime;

CREATE TABLE IF NOT EXISTS harness_runtime.event_log (
    id BIGSERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    session_id UUID NOT NULL,
    sequence BIGINT NOT NULL,
    event_type TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    payload JSONB NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id, sequence)
);

CREATE INDEX IF NOT EXISTS event_log_session_sequence_idx
    ON harness_runtime.event_log (session_id, sequence);
