# ADR-0004: PostgreSQL is the primary runtime source of truth

## Status

Accepted.

## Context

The harness needs durable runtime state for sessions, events, tool observations, usage, recovery, approvals, and future projections. JSON files and ad hoc local state are useful for fixtures and export, but they do not provide enough transactional safety for a long-running runtime.

## Decision

PostgreSQL is the primary V0 runtime source of truth. The runtime records EventLog entries in PostgreSQL and derives observable state from replay.

SQLite, JSONL, and fixture files are allowed only for tests, exports, snapshots, or future lightweight modes. They are not the V0 source-of-truth path.

## Consequences

- Local development requires a reachable PostgreSQL instance.
- Migrations live in source control.
- Event append behavior must be transactionally safe.
- Development-only Kubernetes manifests that use ephemeral storage must be documented as disposable.

## Enforcement

Runtime code must not introduce a second authoritative state store for sessions, events, policy outcomes, tool observations, recovery reports, or usage facts.
