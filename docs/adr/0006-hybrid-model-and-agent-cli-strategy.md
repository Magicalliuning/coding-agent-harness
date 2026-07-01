# ADR-0006: Use a hybrid model API and governed agent CLI strategy

## Status

Accepted.

## Context

The harness should learn from mature coding-agent systems such as Codex, Claude Code, OpenCode, OpenClaw-style harnesses, Hermes-style harnesses, and other provider or open-source agent tools. Some of those systems expose direct model APIs. Others expose powerful CLI or app-server surfaces that already include repository understanding, tools, skills, approvals, and task execution behavior.

If this project only wraps external CLIs, it becomes a supervisor and does not own its runtime semantics. If it only calls raw model APIs, it loses the practical capability surface of mature coding agents. The project needs both: runtime sovereignty and the ability to govern external workers.

## Decision

V0 uses a hybrid strategy.

The harness owns a minimal internal agent loop:

- context compilation
- model routing
- tool call intent parsing
- policy authorization
- tool execution through Tool Runtime
- observation recording
- self-recovery orchestration
- token and budget recording

The harness also supports governed external agent CLI lanes. External agents such as Codex CLI, Claude Code, OpenCode, Gemini CLI, Qwen Code, and similar tools are treated as worker lanes or capability providers, not as the source of truth for the harness runtime.

Both internal model turns and external agent lanes must obey:

- ADR-0001 EventLog source-of-truth discipline
- ADR-0002 Policy Gate and Tool Runtime boundaries where the harness executes actions
- ADR-0003 bounded self-recovery autonomy
- ADR-0004 PostgreSQL runtime SoT
- token, budget, approval, and audit recording

## Boundaries

Internal direct model support is required for V0.

External agent CLI support is also part of the product direction, but V0 can start with one governed lane before expanding to many.

External agent lanes may be allowed to run in restricted worktrees or sandboxes. Their outputs must be captured as observations and associated with session, task, token, budget, and evidence records.

The harness must not rely on external agent chat history or UI state as its authoritative runtime state.

## Consequences

- The project remains a real agent harness runtime rather than only a CLI supervisor.
- Mature external coding tools can still be used as capability sources.
- Worker lanes need clear contracts for stdin/stdout/event capture, worktree isolation, cancellation, timeout, and budget control.
- Token and usage attribution must distinguish internal model calls from external CLI-reported or locally estimated usage.
- Some external tools may not expose enough structured telemetry; those integrations must be marked lower confidence until better adapters exist.

## Non-goals

- V0 does not need to support every provider or CLI.
- V0 does not need automatic model routing across all providers.
- External CLI lanes do not bypass the harness's EventLog, approval, budget, or audit model.
