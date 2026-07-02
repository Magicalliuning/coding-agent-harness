# V0 Acceptance Manual QA

V0 is accepted by one observable self-recovering coding task through the CLI and PostgreSQL EventLog.

## Environment

- Rust toolchain for workspace edition 2024.
- `cargo` and `git` on `PATH`.
- A reachable PostgreSQL database.
- `HARNESS_DATABASE_URL` set to that database, or pass `--database-url` on every CLI command.

For WSL/k3s PostgreSQL, see `docs/development/postgresql.md`.

## Fixture

Use `fixtures/v0-acceptance` as the source fixture. Run acceptance against a copy so the generated `.harness/fake-agent-turn.md` file does not dirty the source fixture.

```powershell
$fixture = Join-Path $env:TEMP "coding-agent-harness-v0-acceptance"
Remove-Item -Recurse -Force $fixture -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $fixture | Out-Null
Copy-Item -Recurse -Path "fixtures/v0-acceptance/*" -Destination $fixture

git -C $fixture init
git -C $fixture add AGENTS.md Cargo.toml src/lib.rs
git -C $fixture -c user.name="Coding Agent Harness QA" -c user.email="harness-qa@example.invalid" commit -m "initial fixture"
```

## Commands

```powershell
cargo run -p harness-cli -- migrate --database-url $env:HARNESS_DATABASE_URL

$start = cargo run -p harness-cli -- session start --repo $fixture --database-url $env:HARNESS_DATABASE_URL
$sessionId = ($start | Select-String '^session_id=').Line.Split('=')[1]

cargo run -p harness-cli -- session recover-fixture $sessionId `
  --database-url $env:HARNESS_DATABASE_URL `
  --task "run the V0 acceptance self-recovery fixture" `
  --max-bytes 4096 `
  --focus agent `
  --max-recovery-rounds 2 `
  --max-repair-bytes 4096
```

## Expected Output

The `recover-fixture` command should report:

- `recovery_classification=fixture_missing_recovery_marker`
- `recovery_stop_reason=recovered`
- `verification_decision=allow`
- `verification_executed=true`
- `verification_exit_code=0`
- `diff_files_changed=1`
- `diff_paths=.harness/fake-agent-turn.md`
- `event_replay_total=22`
- `event_replay_last=recovery.stopped`
- `token_prompt=...`
- `token_completion=...`
- `token_total=...`
- `token_max_output=256`
- `final_state=pending_commit_approval`
- `event_count=22`

The fixture copy should contain `.harness/fake-agent-turn.md` with `recovered=true`.

PostgreSQL should contain the session EventLog entries for session start, context compilation, model request and decision, policy decisions, tool observations, recovery classification, recovery plan, repair attempt, diff summary, commit approval pending, and recovery stopped.

## Known Limits

- This V0 gate uses the internal deterministic fake-model loop, not a real LLM.
- The real Codex CLI manual gate is documented separately in `docs/development/local-codex-cli-acceptance.md`, including the queue-driven Task lease path.
- The harness stops at `pending_commit_approval`; it does not commit or push user code.
- Self-recovery is intentionally bounded to the configured recovery rounds and repair budget.
- PostgreSQL is required for the runtime source of truth; SQLite/JSONL export is not the V0 runtime path.
