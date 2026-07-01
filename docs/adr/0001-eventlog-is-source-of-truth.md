# ADR-0001: EventLog is the source of truth

## Status

Accepted.

## Context

The harness must be able to explain and replay what happened during a coding-agent session. Directly mutating derived state tables without a durable event trail would make recovery, auditing, token attribution, and future UI projections unreliable.

## Decision

Runtime facts are recorded first as append-only EventLog entries. Session state, task state, UI state, reports, token summaries, recovery reports, and eval traces are derived from EventLog replay.

The EventLog records at least these classes of facts:

- session lifecycle
- context compilation
- model requests and decisions
- tool call intents
- policy decisions
- tool observations
- worker lane requests, states, and observations
- verification results
- recovery classification, planning, repair, and stop reasons
- diff summaries and commit approval waits

## Consequences

- New runtime capabilities must add events before adding projections.
- Projections are rebuildable and must not become competing sources of truth.
- Event schema changes require explicit versioning and migration strategy.
- Event append ordering must be concurrency-safe per session.

## Enforcement

- All mutating runtime paths must append intent, decision, and observation events where applicable.
- Tests should assert observable EventLog sequences rather than private agent-loop internals.
