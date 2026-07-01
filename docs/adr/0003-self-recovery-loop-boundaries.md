# ADR-0003: Self-Recovery Loop has bounded autonomy

## Status

Accepted.

## Context

Self-recovery is a core capability of this harness runtime. The system should not only report a failed task; it should capture the failure, classify it, inspect relevant context, propose a repair, apply small changes when allowed, verify the result, and record what happened.

However, self-recovery is also the highest-risk loop in the system. An unbounded loop can burn tokens, repeatedly mutate the wrong files, hide a bad fix behind a passing narrow test, rewrite policy to permit itself, or persist false lessons into memory.

## Decision

The first version of Self-Recovery Loop has bounded autonomy.

It may automatically:

- classify failures
- read relevant repo context, EventLog history, diagnostics, and docs
- create a minimal recovery plan
- apply small scoped patches through Policy Gate and Tool Runtime
- run configured verification commands such as tests, lint, build, or smoke checks
- retry at most two recovery rounds per failed task
- produce a recovery report with diff, verification results, token usage, and remaining risk

It must require explicit approval before:

- creating commits
- pushing to a remote
- changing hooks, policy, or permission profiles
- writing durable memory
- deleting files
- performing broad refactors
- running high-risk shell, network, or remote-machine actions
- increasing its own retry, budget, or permission limits

Every recovery action must be represented in EventLog and must obey ADR-0002.

## Consequences

- The runtime can recover from routine implementation failures without becoming an unattended high-privilege actor.
- Human review remains the boundary for durable history, remote publication, and long-lived memory.
- Recovery reports become useful inputs for future eval traces and failure memory candidates.
- Some recoverable tasks will stop early and ask for approval instead of finishing automatically.
- The runtime needs explicit retry counters, budget accounting, and stop reasons in session state.

## Non-goals

- Self-recovery is not self-modification of the harness runtime.
- Self-recovery is not permission escalation.
- Self-recovery is not a guarantee that the produced fix is correct; it produces evidence and asks for review when the boundary is reached.
