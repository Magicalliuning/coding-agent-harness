# Context Map

This file is the first domain entry point for agents working in this repository. It maps the current V0 runtime areas to the code, documentation, and acceptance evidence that govern them.

## Release Baseline

- Current baseline: `docs/releases/v0.0.1.md`
- Acceptance gate: `docs/development/v0-acceptance.md`
- PRD: `docs/prd/v0-self-recovering-agent-runtime.md`
- Next tranche PRD: `docs/prd/v0.1-v0.3-runtime-expansion.md`
- Domain language: `CONTEXT.md`

## Runtime Core

- Crates: `crates/harness-runtime`, `crates/harness-events`, `crates/harness-db`
- Governing ADRs: `docs/adr/0001-eventlog-as-source-of-truth.md`, `docs/adr/0003-self-recovery-loop-boundaries.md`, `docs/adr/0004-postgresql-first-runtime-sot.md`
- Acceptance: `docs/development/v0-acceptance.md`, `fixtures/v0-acceptance`

## Policy and Tool Runtime

- Crates: `crates/harness-policy`, `crates/harness-tools`
- Governing ADR: `docs/adr/0002-policy-gate-and-tool-runtime-boundary.md`
- Core invariant: models, skills, MCP servers, hooks, recovery loops, and external worker lanes may propose tool intents, but only Tool Runtime executes approved actions.

## Context and Skills

- Crate: `crates/harness-context`
- Entry files: `AGENTS.md`, `CONTEXT-MAP.md`
- Skill metadata roots: `.codex/skills`, `.agents/skills`, `skills`
- Governing ADRs: `docs/adr/0005-v0-rust-workspace-crate-boundaries.md`, `docs/adr/0008-v0-end-to-end-acceptance.md`

## Models and Worker Lanes

- Crate: `crates/harness-models`
- Runtime integration: `crates/harness-runtime`
- Governing ADRs: `docs/adr/0006-hybrid-model-and-agent-cli-strategy.md`, `docs/adr/0007-codex-cli-first-external-agent-lane.md`
- Current V0 lane: deterministic fake model plus Codex CLI fixture-adapter boundary.

## CLI Surface

- Crate: `crates/harness-cli`
- V0 commands: `session start`, `session show`, `session verify-command`, `session compile-context`, `session fake-turn`, `session coding-task`, `session recover-fixture`
- Development checks: `docs/development/checks.md`

## Out-of-scope for V0

- Web dashboard
- iOS client
- IDE plugin
- Long-running remote daemon
- Full MCP client/server support
- Production Nginx automation
- Real Codex CLI execution
- Automatic commit or push
