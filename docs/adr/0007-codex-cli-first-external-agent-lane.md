# ADR-0007: Codex CLI is the first governed external agent lane

## Status

Accepted.

## Context

ADR-0006 chooses a hybrid strategy: the harness owns an internal minimal agent loop while also governing external coding-agent CLIs as worker lanes. The first external lane should validate the integration contract without forcing support for every mature agent tool at once.

Codex CLI is a strong first lane because the current project is being developed through Codex, the local environment already has GitHub and Codex-oriented workflow assumptions, and Codex concepts such as approvals, sandboxing, usage, MCP, and `AGENTS.md` align closely with this harness's runtime boundaries.

## Decision

Codex CLI is the first governed external agent lane.

The V0 Codex lane should treat Codex as an external worker, not as the harness source of truth. The harness owns:

- task creation
- EventLog records
- worktree allocation
- budget and retry limits
- approval state
- captured observations
- recovery boundaries
- final task status

The Codex lane should run inside a controlled workspace or worktree and produce captured output, status, diffs, and usage observations that are appended to EventLog.

## Boundaries

The Codex lane must distinguish:

- usage reported by Codex or official surfaces
- locally estimated usage
- unknown or unavailable usage

Codex internal chat history, UI state, or remote service state is not authoritative for this harness. It may be referenced as evidence, but the harness EventLog remains the source of truth.

Codex lane execution must support cancellation, timeout, budget stop, and human approval handoff.

Claude Code, OpenCode, Gemini CLI, Qwen Code, and other agent tools remain future lanes. Their abstractions should not be blocked by Codex-specific assumptions.

## Consequences

- The first external worker integration targets the toolchain most aligned with the current development environment.
- The worker lane contract can be validated before supporting many providers.
- Codex-specific telemetry gaps must be represented explicitly instead of silently treated as authoritative.
- The harness must design lane metadata generically enough that Claude Code and OpenCode can follow later.

## Non-goals

- V0 does not need to support every Codex feature.
- Codex CLI does not replace the internal agent loop.
- Codex lane support does not permit Codex to bypass Policy Gate, budget controls, worktree isolation, or EventLog recording.
