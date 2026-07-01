# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Layout

This repo uses a multi-context documentation layout.

- **`CONTEXT-MAP.md`** at the repo root is the entry point. It points to one `CONTEXT.md` per context.
- **`docs/adr/`** contains system-wide architectural decisions.
- **`src/<context>/docs/adr/`** or an equivalent context-local ADR directory may contain context-specific decisions once the codebase has clear subprojects.

## Before exploring, read these

- **`CONTEXT-MAP.md`** at the repo root if it exists. Read each context entry relevant to the topic.
- The relevant context-specific **`CONTEXT.md`** files named by `CONTEXT-MAP.md`.
- **`docs/adr/`** for decisions that touch the area you're about to work in.
- Context-local ADR directories for decisions scoped to a subsystem.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The `/domain-modeling` skill creates them lazily when terms or decisions actually get resolved.

## File structure

```text
/
├── CONTEXT-MAP.md
├── docs/adr/                          # system-wide decisions
└── src/
    ├── runtime/
    │   ├── CONTEXT.md
    │   └── docs/adr/                  # context-specific decisions
    ├── model-gateway/
    │   ├── CONTEXT.md
    │   └── docs/adr/
    └── performance/
        ├── CONTEXT.md
        └── docs/adr/
```

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in the relevant `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal -- either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0007 (event-sourced orders) -- but worth reopening because..._
