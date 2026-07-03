# Coding Agent Harness

This context defines the project language for the Rust coding-agent harness
runtime. It exists so future planning, PRDs, issues, and agents use the same
terms for runtime ownership, worker lanes, approvals, and audit evidence.

## Language

**Private Coding-Agent Runtime**:
A self-owned runtime that accepts coding tasks, governs model and tool
execution, records replayable evidence, produces reviewable diffs, and owns the
approval-to-commit path.
_Avoid_: Chat shell, IDE wrapper, external agent source of truth

**Evidence-Producing Participant**:
A model provider, external CLI, MCP server, skill, hook, subagent, or client
surface that may contribute observations, outputs, diffs, or decisions while
the harness remains the runtime source of truth.
_Avoid_: Runtime owner, independent commit authority, hidden state owner

**Local Governed Codex CLI Worker**:
A local Codex CLI execution treated as a governed worker lane for one harness
task. It is an evidence-producing worker, not the runtime source of truth.
_Avoid_: Codex Cloud worker, remote daemon, Codex source of truth

**Task Worktree**:
A git worktree allocated for one harness task so a worker lane can edit files
without sharing the user's current working tree or another worker's workspace.
_Avoid_: Shared workspace, current repo by default

**One-Shot Worker Run**:
A non-interactive worker execution that receives one task input, runs once in a
Task Worktree, emits observations, and exits with a final worker status.
_Avoid_: Interactive chat session, long-running daemon, background conversation

**Commit Handoff**:
The harness-owned step that turns an approved Task Worktree diff into durable
git history. Workers propose diffs; the harness records approval and performs
the commit operation.
_Avoid_: Worker-owned commit, auto-push, oral approval

**Task**:
A schedulable runtime work unit with its own input, repository target, worker
lane, budget, Task Worktree, status, diff, and approval outcome.
_Avoid_: Session payload, CLI invocation, worker run

**Session**:
A runtime context that starts from a user or client interaction and aggregates
tasks, events, context, approvals, and observations.
_Avoid_: Task, worker run, UI chat history

**Vertical Acceptance Path**:
A release slice that proves one observable runtime path end to end through CLI,
PostgreSQL EventLog, policy, worker execution, recovery or approval state, and
replayable evidence.
_Avoid_: Schema-first milestone, abstraction-only milestone, dashboard-only milestone

**Lane-Level Governance**:
Policy control over whether, where, and with what limits an external worker lane
may run, plus capture of its outputs and resulting diff. It does not require
intercepting every internal tool call made by that external worker in V0.1.
_Avoid_: Internal tool interception, external session source of truth

**Manual Lane Acceptance**:
A release check that invokes a locally installed and authenticated external
worker CLI when available, while automated tests use deterministic fixtures.
Unavailable local CLIs must produce an explicit skipped reason.
_Avoid_: CI-only real CLI dependency, silent skip

**Approval State Machine**:
The runtime-owned states and events that move a task from pending commit
approval through approval, rejection, commit handoff, and final commit outcome.
_Avoid_: UI-only approval, unrecorded approval, worker-owned approval

**Task Lease**:
A PostgreSQL-backed claim that gives one worker temporary ownership of a queued
task. Expired leases allow the task to be retried or reassigned.
_Avoid_: In-memory lock, distributed scheduler, permanent worker ownership

**First Real Lane**:
The first external worker lane implemented against a real local CLI while the
runtime keeps the lane model generic enough for later tools.
_Avoid_: Multi-provider framework, provider routing, every lane at once
