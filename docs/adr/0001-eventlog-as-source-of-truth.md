# ADR-0001: EventLog is the source of truth

## Status

Accepted.

## Context

This project is a Rust agent harness runtime, not only a control panel or IDE shell. The runtime must support agent execution, tool calls, approvals, failure recovery, audits, replay, token accounting, and multiple clients such as CLI, TUI, Web, iOS, IDE, and webhooks.

If session state, UI state, tool outputs, model decisions, and recovery attempts are stored as separate independent truths, the system becomes hard to audit and unsafe to run unattended. Self-recovery also becomes unreliable because the runtime cannot explain what failed, which action caused it, which budget was consumed, or which recovery step changed the workspace.

## Decision

EventLog is the only source of truth for agent sessions.

All meaningful runtime facts must be appended as events before being used to derive state or displayed through a client:

- user messages and task creation
- model requests, model decisions, and model responses
- context compilation inputs and budget decisions
- tool calls, policy decisions, approvals, denials, and observations
- shell output, file changes, git state, test results, and diagnostics
- failures, classifications, recovery plans, repair attempts, reviewer results, and escalations
- token usage, estimated cost, rate-limit observations, and budget state
- memory candidates, accepted memories, and rejected memory writes
- session completion, cancellation, pause, resume, and replay metadata

Session state, UI views, audit exports, recovery summaries, token dashboards, and eval traces are derived from EventLog replay.

## Consequences

- Agent behavior can be replayed and inspected after the fact.
- Self-recovery has structured evidence for failure classification and repair decisions.
- Multiple clients can share the same runtime without owning business state.
- Token and cost controls can be enforced from recorded usage and budget events.
- Tool execution and policy checks become auditable instead of hidden side effects.
- The storage and event schema need careful versioning from the beginning.
- The runtime must avoid direct state mutation paths that bypass EventLog.

## Non-goals

- EventLog is not a free-form text log.
- EventLog is not the UI event stream, though clients may subscribe to derived event views.
- EventLog does not replace structured indexes, caches, or projections; those are derived artifacts and can be rebuilt.
