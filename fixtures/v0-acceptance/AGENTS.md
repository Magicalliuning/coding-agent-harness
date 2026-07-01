Use deterministic recovery fixtures for V0 acceptance.

The harness should write `.harness/fake-agent-turn.md`, run verification,
recover by appending `recovered=true`, and stop at pending commit approval.
