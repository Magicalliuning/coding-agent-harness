# ADR-0003: Self-Recovery Loop has bounded autonomy

## Status

Accepted.

## Context

The harness should be able to recover from routine failures without becoming an uncontrolled autonomous process. A recovery loop that can retry indefinitely, widen its own permissions, or silently persist lessons would turn a coding assistant into an unsafe background actor.

## Decision

Self-Recovery Loop is allowed to classify failures, read bounded context, propose minimal repair plans, apply small policy-approved patches, run verification, and stop with an auditable report.

For V0, self-recovery is bounded by:

- maximum recovery rounds
- repair byte budget
- policy-approved file patch scope
- verification command policy
- pending commit approval rather than automatic commit

## Consequences

- Self-recovery must stop with a clear stop reason.
- Recovery does not commit, push, change policy, change hooks, increase limits, delete files, or write durable memory without explicit approval.
- Recovery reports are evidence, not final authority.

## Enforcement

Recovery events must include classification, plan, repair attempt, verification outcome, stop reason, retry count, and final state. Tests must prove success, max-round stop, and repair-budget stop behavior.
