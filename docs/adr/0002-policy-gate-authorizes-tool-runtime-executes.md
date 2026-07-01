# ADR-0002: Policy Gate authorizes, Tool Runtime executes

## Status

Accepted.

## Context

Coding agents, skills, MCP servers, hooks, recovery loops, and external worker lanes can all propose actions. If any of them can directly mutate files, run shell commands, change Git state, or call external tools, the harness loses its central safety boundary.

## Decision

Policy Gate is the only authorization surface. Tool Runtime is the only execution surface.

Models, skills, MCP servers, hooks, self-recovery loops, and external worker lanes may only produce tool intents. A tool intent must be recorded before policy evaluation. A policy decision must be recorded before execution. Tool Runtime records an observation after execution or after a governed non-execution result.

## Consequences

- Mutating actions require explicit policy evaluation.
- High-risk shell, network, deletion, Git, policy, hook, remote, and permission actions must be denied or require human approval.
- Tools must carry risk metadata, mutation flags, path scope, capability origin, and decision reason as the policy model matures.
- Worker lanes are governed tools, not trusted runtime owners.

## Enforcement

The canonical path is:

```text
proposal -> tool intent event -> policy decision event -> Tool Runtime -> observation event
```

Any code path that directly executes a mutating action without this chain violates this ADR.
