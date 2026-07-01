# ADR-0005: V0 Rust workspace crate boundaries

## Status

Accepted.

## Context

The project is a clean rebuild of a Rust agent harness runtime. It should reuse proven concepts from prior experiments without copying their history wholesale. The first implementation needs enough separation to keep EventLog, PostgreSQL storage, policy, tools, model routing, context compilation, runtime orchestration, and CLI concerns independent.

At the same time, V0 should not pre-build every future client or marketplace surface. Web, iOS, IDE, MCP server mode, and plugin marketplace should come after the core runtime loop is proven.

## Decision

The V0 Rust workspace should start with these crates:

```text
crates/harness-core        # domain types, ids, errors, config primitives
crates/harness-events      # EventLog schema, append/replay, event versioning
crates/harness-db          # PostgreSQL migrations, repositories, projections
crates/harness-policy      # Policy Gate, risk model, approval requirements
crates/harness-tools       # Tool Registry + Tool Runtime interfaces
crates/harness-models      # Model Router and provider-normalized types
crates/harness-context     # Context Compiler, AGENTS/CONTEXT/SKILL selection
crates/harness-runtime     # Agent Loop + Self-Recovery orchestration
crates/harness-cli         # CLI/TUI entrypoint for V0
```

V0 should not start with these crates:

```text
crates/harness-web
crates/harness-ios
crates/harness-ide
crates/harness-mcp-server
crates/harness-plugin-marketplace
```

Those surfaces should be added only after the PostgreSQL EventLog, Policy Gate, Tool Runtime, Agent Loop, and Self-Recovery Loop work through the CLI path.

## Dependency direction

The intended dependency direction is:

```text
harness-cli
  -> harness-runtime
      -> harness-context
      -> harness-models
      -> harness-tools
      -> harness-policy
      -> harness-db
          -> harness-events
          -> harness-core
      -> harness-events
      -> harness-core
```

Domain types in `harness-core` must not depend on runtime, database, model provider, CLI, or tool execution details.

## Consequences

- The core runtime can be tested without committing to a UI stack.
- Event schema and database persistence stay separated from agent orchestration.
- Tool execution and authorization remain independently testable.
- Future clients can reuse the same runtime rather than reimplementing agent loops.
- There will be more crates than a toy CLI would need, but each crate maps to a durable runtime boundary.

## Non-goals

- This ADR does not create the workspace.
- This ADR does not freeze every module forever; crates may merge if a boundary proves artificial.
- This ADR does not require production-ready implementations in every crate before the first CLI smoke path.
