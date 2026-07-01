# ADR-0002: Policy Gate authorizes, Tool Runtime executes

## Status

Accepted.

## Context

The harness runtime will accept tool requests from models, skills, MCP servers, subagents, hooks, and recovery loops. Many of those sources are untrusted or only partially trusted. Some tool requests mutate files, run shell commands, change git state, reach the network, write memory, or affect remote machines.

If any source can execute tools directly, the runtime cannot enforce permissions, budgets, approvals, path boundaries, secret protection, or audit guarantees. Self-recovery would be especially risky because a failed task could enter a loop that repeatedly mutates the workspace without a single authorization boundary.

## Decision

Policy Gate is the only authorization surface.

Tool Runtime is the only execution surface.

Models, skills, MCP servers, subagents, hooks, and recovery loops may propose tool call intents. They must not directly execute mutating file, shell, git, network, memory, or remote actions.

Every tool call intent must flow through this sequence:

```text
Model / Skill / MCP / Subagent / Hook / Recovery Loop
        -> Tool Call Intent
        -> EventLog append: intent proposed
        -> Policy Gate: allow / ask / deny
        -> EventLog append: policy decision
        -> Tool Runtime: execute allowed calls only
        -> EventLog append: observation
```

Tool Runtime may execute only calls that carry an allowed Policy Gate decision. Denied calls are recorded and not executed. Calls that require approval must wait for an explicit approval event before execution.

## Consequences

- Mutation policy is centralized and auditable.
- Skill and MCP capabilities can be added without giving them direct process authority.
- Self-recovery can attempt repairs while still obeying permissions, budgets, and approval gates.
- Tool observations become structured inputs for the next agent turn and for replay.
- Tool schemas need risk metadata, mutation flags, capability origin, and policy requirements.
- Bypassing Tool Runtime is a correctness and security bug.

## Non-goals

- Policy Gate does not decide whether a task is a good idea; it decides whether a proposed action is allowed.
- Tool Runtime does not own product policy; it only executes authorized calls and records observations.
- This ADR does not require every read-only query to be manually approved, but read tools still need schemas, origin tracking, and audit events.
