# ADR-0008: V0 is accepted by an end-to-end self-recovering task

## Status

Accepted.

## Context

The project can easily drift into a broad framework without proving the core harness loop. V0 needs a narrow acceptance path that exercises the runtime's most important invariants through observable behavior.

## Decision

V0 is accepted by one CLI-started, PostgreSQL-backed, end-to-end self-recovering coding task.

The acceptance path must show:

- a session started against a repository
- context compiled from repository instructions and domain context
- deterministic model decision recorded
- tool intent recorded before authorization
- policy decision recorded before execution
- tool observation recorded after execution
- verification run through Tool Runtime
- initial failure classified
- bounded recovery plan recorded
- repair attempt recorded
- successful verification after recovery
- diff summary recorded
- pending commit approval recorded
- recovery stopped with final state
- token summary reported
- event replay summary reported

## Consequences

- V0 does not require Web, iOS, IDE, daemon, full MCP, real Codex execution, or production deployment.
- The acceptance gate is intentionally fake-model based and deterministic.
- The harness must stop at pending commit approval rather than committing automatically.

## Enforcement

`docs/development/v0-acceptance.md` is the manual QA gate. CLI and runtime tests should assert the same observable state: event count, last event, recovery stop reason, diff path, token summary, and pending commit approval.
