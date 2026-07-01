# PRD: V0 Self-Recovering Agent Runtime

## Problem Statement

The user wants a clean public rebuild of a Rust coding-agent harness runtime. The product should not be another IDE plugin, chat wrapper, or static control panel. It should provide the runtime order around coding agents: durable events, policy-gated tool execution, model and worker orchestration, token accounting, failure recovery, and auditable task results.

The user already has related experiments covering runtime-first IDE structure, AI Ops source-of-truth modeling, remote control planes, skills, data governance, and Rust service baselines. The new project should reuse those proven concepts without copying old repository history or inheriting accidental complexity.

The core pain is that mature tools such as Codex, Claude Code, OpenCode, OpenClaw-style harnesses, Hermes-style harnesses, and other coding-agent systems expose useful capabilities, but they do not give the user one personal, Rust-based runtime that owns task state, safety policy, budget control, self-recovery, and replayable evidence across tools.

## Solution

Build a PostgreSQL-backed Rust agent harness runtime whose V0 proves one vertical path: a CLI-started coding session that records every meaningful runtime fact in EventLog, compiles bounded context from repository instructions, routes work through an internal model loop or a governed Codex CLI worker lane, authorizes tool calls through Policy Gate, executes approved actions through Tool Runtime, runs verification, enters a bounded Self-Recovery Loop on failure, and outputs an auditable recovery report with diff, token ledger, event replay summary, and pending human approval for commit.

The runtime is the product core. CLI, TUI, Web, iOS, IDE, webhook, and future clients are surfaces over the runtime, not independent owners of agent logic.

## User Stories

1. As the project owner, I want a clean Rust workspace for the new harness, so that the public project can evolve without old experimental history.
2. As the project owner, I want EventLog to be the source of truth, so that every agent decision and tool result can be replayed.
3. As the project owner, I want PostgreSQL to store runtime facts, so that sessions, tasks, approvals, usage, and eval traces are queryable from the beginning.
4. As the project owner, I want derived state to be rebuildable from EventLog, so that UI and projections do not become competing truths.
5. As a CLI user, I want to start a coding session against a repository, so that I can drive the first version without waiting for Web or iOS clients.
6. As a CLI user, I want the runtime to read repository instructions, so that agent behavior follows the repo's rules.
7. As a CLI user, I want the runtime to understand multi-context documentation, so that it can select the right domain context for a task.
8. As a CLI user, I want the runtime to show a task's current state, so that I know whether it is planning, executing, recovering, waiting, or complete.
9. As a CLI user, I want a final recovery report, so that I can understand what changed and why.
10. As a CLI user, I want commit to require approval, so that self-recovery does not silently create durable history.
11. As a runtime developer, I want crate boundaries for core, events, database, policy, tools, models, context, runtime, and CLI, so that the system can grow without mixing concerns.
12. As a runtime developer, I want EventLog schema versioning, so that future event formats can migrate safely.
13. As a runtime developer, I want append and replay primitives, so that tests and evals can reconstruct session behavior.
14. As a runtime developer, I want PostgreSQL migrations, so that the runtime schema can evolve under source control.
15. As a runtime developer, I want repositories for runtime data, so that storage details stay out of orchestration code.
16. As a runtime developer, I want session projections, so that clients can query current task state without replaying everything themselves.
17. As a runtime developer, I want a Policy Gate, so that every mutating action has one authorization boundary.
18. As a runtime developer, I want Tool Runtime to be the only execution surface, so that models, skills, MCP, hooks, and worker lanes cannot bypass policy.
19. As a runtime developer, I want tool schemas with risk metadata, mutation flags, and capability origin, so that policy decisions can be precise.
20. As a runtime developer, I want built-in file, shell, git, and verification tools, so that V0 can complete a real coding task.
21. As a runtime developer, I want all tool observations recorded, so that the next agent turn and later audits see the same facts.
22. As a runtime developer, I want model-provider-normalized types, so that internal model calls do not leak provider-specific shapes through the runtime.
23. As a runtime developer, I want an internal minimal agent loop, so that the harness owns runtime semantics instead of only supervising external CLIs.
24. As a runtime developer, I want external agent CLI lanes, so that mature tools such as Codex CLI can be governed as workers.
25. As a runtime developer, I want Codex CLI as the first governed external lane, so that V0 can validate worker governance with the tool closest to the current workflow.
26. As a runtime developer, I want external lane outputs captured as observations, so that worker runs become part of the harness audit trail.
27. As a runtime developer, I want external lane usage confidence recorded, so that official usage, local estimates, and unknown usage are not mixed.
28. As a runtime developer, I want Context Compiler budgeting, so that the model receives relevant instructions without uncontrolled context growth.
29. As a runtime developer, I want Skill discovery and loading boundaries, so that skills can influence workflows without becoming hidden executors.
30. As a runtime developer, I want MCP capability origins reserved in tool metadata, so that future MCP tools fit the same policy model.
31. As a runtime developer, I want Self-Recovery Loop to classify failures, so that repair attempts are based on structured causes.
32. As a runtime developer, I want Self-Recovery Loop to propose minimal repair plans, so that recovery stays scoped.
33. As a runtime developer, I want Self-Recovery Loop to apply only small policy-approved patches, so that it can fix routine failures without broad uncontrolled edits.
34. As a runtime developer, I want Self-Recovery Loop to retry at most two rounds in V0, so that failure recovery cannot loop indefinitely.
35. As a runtime developer, I want durable memory writes to require approval, so that false recovery lessons do not become permanent.
36. As a runtime developer, I want token and budget events, so that every session can report cost and stop before overspending.
37. As a runtime developer, I want stop reasons recorded, so that failures, denials, budget stops, and approval waits are distinguishable.
38. As a runtime developer, I want an eval trace for the V0 scenario, so that later changes can prove they did not break the harness loop.
39. As a security reviewer, I want mutation attempts recorded before execution, so that denied and approved actions are both auditable.
40. As a security reviewer, I want high-risk shell, network, deletion, policy, hook, and remote actions to require approval, so that self-recovery cannot escalate itself.
41. As a security reviewer, I want external content treated as untrusted, so that skills, MCP servers, repos, and model outputs do not gain implicit trust.
42. As a future Web client developer, I want all current state to come from runtime projections, so that a Web UI does not need its own agent logic.
43. As a future iOS client developer, I want approvals and task status to be runtime APIs, so that mobile can become a client without owning execution.
44. As a future IDE client developer, I want IDE integrations to attach to the runtime, so that editor plugins do not run separate agent loops.
45. As a future maintainer, I want V0 scope to exclude polished dashboards and production infrastructure, so that the core harness behavior lands first.

## Implementation Decisions

- The product is a Rust agent harness runtime, not an IDE plugin, static control panel, or raw model wrapper.
- V0 must be implemented as a clean rebuild while reusing concepts from prior projects as design input.
- EventLog is the source of truth for sessions, tool calls, model decisions, approvals, failures, recovery attempts, usage, and completion.
- Session state, UI state, audit exports, token dashboards, and eval traces are derived from EventLog replay.
- PostgreSQL is the primary runtime source of truth for V0.
- PostgreSQL stores EventLog entries, projections, tool intents, policy decisions, approvals, observations, token usage, task queue state, recovery attempts, failure memory candidates, and eval traces.
- SQLite and JSONL are allowed only for tests, fixtures, export, snapshots, or future lightweight modes.
- V0 requires local or development PostgreSQL, but not k3s, cloud database hosting, high availability, replication, backup automation, or multi-node workers.
- The V0 Rust workspace is split into domain/core, events, database, policy, tools, models, context, runtime, and CLI crates.
- Web, iOS, IDE, MCP server mode, and plugin marketplace are future clients or surfaces, not V0 startup crates.
- Policy Gate is the only authorization surface.
- Tool Runtime is the only execution surface.
- Models, skills, MCP servers, subagents, hooks, recovery loops, and external agent lanes may propose tool call intents but must not directly execute mutating actions.
- Tool call intents are recorded before authorization, policy decisions are recorded before execution, and observations are recorded after execution.
- Self-Recovery Loop has bounded autonomy in V0.
- Self-Recovery Loop may classify failures, read context and diagnostics, propose minimal repair plans, apply small approved patches, run verification, retry up to two rounds, and produce recovery reports.
- Self-Recovery Loop requires explicit approval before commit, push, durable memory writes, policy changes, hook changes, permission changes, file deletion, broad refactors, high-risk shell/network/remote actions, or increased limits.
- V0 uses a hybrid model strategy.
- The harness owns a minimal internal agent loop for context compilation, model routing, tool intent parsing, policy authorization, tool execution, observation recording, self-recovery orchestration, and token recording.
- External coding-agent CLIs are governed worker lanes or capability providers, not sources of truth.
- Codex CLI is the first governed external agent lane.
- Codex lane runs in a controlled workspace or worktree, supports cancellation, timeout, budget stop, approval handoff, and captured observations.
- Usage attribution distinguishes official provider or CLI-reported usage, local estimates, and unknown usage.
- Context Compiler selects bounded inputs from repository rules, domain docs, git state, recent events, diagnostics, skills, and failure summaries.
- Skill loading is supported as discovery and explicit load boundaries in V0; marketplaces and self-generating skills are out of scope.
- MCP origin metadata is reserved in tool and policy models, but full bidirectional MCP support is not required for V0.
- The V0 acceptance path is one end-to-end self-recovering coding task started from CLI.
- V0 is not complete unless that path records EventLog, uses PostgreSQL, compiles context, routes through an internal loop or Codex lane, authorizes tools, executes verification, performs bounded recovery on failure, records token usage, and outputs an approval-ready report.

## Testing Decisions

- The highest-value test seam is the CLI end-to-end self-recovering coding task.
- Tests should verify external behavior: recorded events, policy outcomes, workspace diffs, verification results, recovery stop reasons, usage records, and report output.
- Tests should not assert private implementation details of the agent loop when observable EventLog and CLI output are enough.
- Database tests should validate migrations, append-only event writes, replay, projections, and schema version behavior.
- Policy tests should cover allow, ask, deny, high-risk actions, path boundaries, mutation flags, and attempts to bypass Tool Runtime.
- Tool tests should cover built-in shell, file patch, git status/diff, and verification command observations.
- Context Compiler tests should cover repository instruction loading, multi-context selection, budget limits, skill summary selection, and omission of unrelated context.
- Model Router tests should use fake providers before real providers, so the runtime loop is deterministic.
- Codex lane tests should start with adapter contract tests and captured-output fixtures before invoking a real external CLI.
- Self-Recovery tests should use an intentionally failing fixture task that can be fixed by a small patch.
- Self-Recovery tests should prove the two-round retry cap and approval boundaries.
- Token ledger tests should distinguish provider-reported usage, CLI-reported usage, local estimates, and unknown usage.
- Eval trace tests should replay the V0 acceptance scenario and confirm the same final state can be reconstructed.
- Integration tests should run against a local development PostgreSQL instance.
- V0 manual QA must drive the CLI against a test repository and observe the full recovery report, diff, token ledger, event replay summary, and pending approval.

## Out of Scope

- Full IDE implementation.
- Polished Web dashboard.
- iOS client.
- Production k3s deployment.
- Cloud database hosting.
- High availability, replication, and backup automation.
- Multi-machine workers.
- Broad provider support.
- Automatic model routing across all providers.
- Full MCP client/server support.
- Plugin or skill marketplace.
- Self-modifying hooks, policies, or skills.
- Automatic commit or push.
- Long-term unattended production execution.
- Large-scale RAG, knowledge graph, or documentation platform.
- Broad refactors imported wholesale from prior repositories.

## Further Notes

This PRD is governed by the accepted architecture decisions:

- ADR-0001: EventLog is the source of truth.
- ADR-0002: Policy Gate authorizes, Tool Runtime executes.
- ADR-0003: Self-Recovery Loop has bounded autonomy.
- ADR-0004: PostgreSQL is the primary runtime source of truth.
- ADR-0005: V0 Rust workspace crate boundaries.
- ADR-0006: Use a hybrid model API and governed agent CLI strategy.
- ADR-0007: Codex CLI is the first governed external agent lane.
- ADR-0008: V0 is accepted by an end-to-end self-recovering coding task.

The PRD intentionally keeps the first implementation focused on a vertical runtime proof. Future surfaces such as Web, iOS, IDE, richer MCP, additional external agent lanes, and remote daemon control should attach to the same runtime once the V0 path is working.
