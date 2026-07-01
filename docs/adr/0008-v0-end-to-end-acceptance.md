# ADR-0008: V0 is accepted by an end-to-end self-recovering coding task

## Status

Accepted.

## Context

This project should not declare success because a workspace scaffold builds, isolated crates compile, or a dashboard renders static state. The core product promise is a Rust agent harness runtime that can run a coding task, enforce policy, record events, track budget, recover from failures within bounds, and produce an auditable result.

The V0 acceptance standard must force the runtime to exercise the meaningful system boundaries together.

## Decision

V0 is accepted only when an end-to-end self-recovering coding task works through the matching runtime surface.

The acceptance scenario is:

1. A user starts a session from the CLI against a test repository.
2. The runtime stores session and task events in PostgreSQL EventLog.
3. The runtime reads repo instructions such as `AGENTS.md` and `CONTEXT-MAP.md`.
4. The Context Compiler builds a bounded request context.
5. The internal model loop or the governed Codex CLI lane proposes a small code change.
6. Policy Gate authorizes, asks for approval, or denies each tool intent.
7. Tool Runtime applies allowed file changes and runs configured verification commands.
8. If verification fails, Self-Recovery Loop classifies the failure, proposes a small repair, applies allowed changes, and retries at most two rounds.
9. The runtime records token usage, budget state, tool observations, recovery attempts, diffs, and final verification results.
10. The CLI outputs a recovery report, event replay summary, diff summary, token ledger summary, and a pending human approval for commit.

Empty scaffolding, isolated unit success, or a green build without this scenario does not satisfy V0.

## Consequences

- Implementation work must be sliced around a real vertical path.
- Database, runtime, policy, tools, context, model, recovery, and CLI layers must integrate early.
- Test fixtures need at least one intentionally failing coding task.
- The first release may be smaller in surface area, but it must prove the actual harness behavior.
- UI, iOS, IDE, marketplace, and production deployment remain secondary until this path works.

## Non-goals

- V0 does not require a polished dashboard.
- V0 does not require broad provider support.
- V0 does not require unattended commit or push.
- V0 does not require high availability, k3s deployment, or multi-machine workers.
