# ADR-0004: PostgreSQL is the primary runtime source of truth

## Status

Accepted.

## Context

The harness runtime needs durable storage for EventLog, session projections, token usage, approvals, task queue state, recovery attempts, failure memory candidates, and eval traces. This project is intended to become a long-running, remotely observable, multi-client agent runtime rather than a single-process CLI toy.

The user's existing AI Ops workbench already uses a PostgreSQL source-of-truth model with evidence chains and worker lanes. Reusing that architectural direction makes the runtime easier to extend toward remote control, dashboards, multiple agents, and structured audits.

## Decision

PostgreSQL is the primary runtime source of truth for V0.

The following runtime data should be modeled in PostgreSQL from the start:

- EventLog entries
- session and task state projections
- tool call intents, policy decisions, approvals, and observations
- token usage, budget state, cost estimates, and rate-limit observations
- self-recovery attempts, retry counters, stop reasons, and recovery reports
- failure memory candidates and memory review state
- eval traces and replay metadata
- task queue and scheduler state

SQLite and JSONL may be used for tests, fixtures, import/export, offline snapshots, or a future lightweight mode. They are not the primary runtime storage path for V0.

## Boundaries

V0 requires a local or development PostgreSQL instance.

V0 does not require:

- k3s deployment
- cloud database hosting
- high availability
- replication
- backup automation
- multi-node workers

Those can be added after the core runtime loop, policy boundary, self-recovery loop, and observability model are proven.

## Consequences

- The runtime can query, replay, and audit sessions with structured storage from the beginning.
- Token dashboards, approval history, task queues, and eval traces can share one durable data model.
- Schema migration becomes part of the core engineering baseline.
- Local development needs a reliable PostgreSQL bootstrap path.
- Tests need database lifecycle tooling.
- The open-source setup must document a low-friction local database path, such as a dev container, Docker Compose, or `xtask` helper.

## Non-goals

- PostgreSQL is not a substitute for EventLog discipline; EventLog remains the source-of-truth model inside PostgreSQL.
- PostgreSQL-first does not mean infra-first. The first release should still avoid production k3s, HA, and cloud deployment scope.
- PostgreSQL does not make derived projections authoritative; projections remain rebuildable from EventLog.
