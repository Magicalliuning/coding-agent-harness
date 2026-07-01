# ADR-0006: Use a hybrid model API and governed agent CLI strategy

## Status

Accepted.

## Context

The harness should own runtime semantics while still being able to use mature external coding-agent tools. Relying only on external CLIs would make them the source of truth. Relying only on direct model APIs would delay validation of the user's real workflow around Codex-style tools.

## Decision

V0 uses a hybrid strategy:

- an internal deterministic fake model path for repeatable runtime tests
- future direct model-provider paths through provider-normalized types
- governed external agent CLI lanes for tools such as Codex CLI

External agent CLIs are worker lanes or capability providers. They are not runtime owners. Their inputs, outputs, status, timeout, cancellation, stdout/stderr, usage confidence, and observations are recorded by the harness.

## Consequences

- The runtime can be tested deterministically before real model spend.
- Real external agents can be integrated without surrendering policy, budget, and audit control.
- Usage attribution must distinguish official provider reports, CLI-reported values, local estimates, and unknown usage.

## Enforcement

Worker lanes must be invoked through a governed tool path. They must produce EventLog entries for request, policy decision, state changes, and observation. They must not directly update durable runtime state outside EventLog.
