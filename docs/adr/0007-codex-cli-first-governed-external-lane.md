# ADR-0007: Codex CLI is the first governed external agent lane

## Status

Accepted.

## Context

The user's current workflow relies heavily on Codex-style coding agents. The harness should validate how an external coding agent can be governed without letting that external tool become the source of truth.

## Decision

Codex CLI is the first external worker lane modeled by the harness.

For V0, the lane is represented by a contract/schema/fixture adapter. A real Codex CLI process is out of scope for the V0 acceptance gate. The governed lane model still records:

- lane request
- lane kind
- task
- workspace and optional worktree
- timeout
- cancellation state
- prompt/output/stdout budget
- policy decision
- state transitions
- observation
- usage confidence

## Consequences

- The harness proves governance before invoking a real external CLI.
- Future real Codex execution should fit the existing request/state/observation event model.
- Timeout, cancellation, stdout truncation, and budget behavior can be tested deterministically.

## Enforcement

A Codex worker lane must never write directly to EventLog or mutate runtime state outside the harness. It must return observations to Tool Runtime, which records governed events.
