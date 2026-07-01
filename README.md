# Coding Agent Harness

Rust-first coding-agent harness runtime. The V0 goal is a PostgreSQL-backed runtime that records EventLog entries, policy decisions, tool observations, token usage, and recovery attempts while driving a CLI-started self-recovering coding task.

## Current Baseline

The V0.0.1 runtime baseline is documented in
`docs/releases/v0.0.1.md`. It captures the accepted scope, verification gate,
known limits, and next tranche boundary for work after the first vertical
self-recovery proof.

## Development Checks

```powershell
cargo fmt --all --check
cargo test --workspace
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The CLI baseline can be smoke-tested with:

```powershell
cargo run -p harness-cli -- --version
cargo run -p harness-cli -- doctor
```

With PostgreSQL available:

```powershell
cargo run -p harness-cli -- migrate --database-url $env:HARNESS_DATABASE_URL
cargo run -p harness-cli -- session start --repo . --database-url $env:HARNESS_DATABASE_URL
```

The V0 manual QA gate is documented in `docs/development/v0-acceptance.md`.
It runs the CLI against `fixtures/v0-acceptance`, records PostgreSQL EventLog
entries, exercises the bounded self-recovery loop, and stops at pending commit
approval.

## PostgreSQL Development Baseline

Copy `.env.example` to `.env` and start a local PostgreSQL instance. If Docker Compose is available:

```powershell
docker compose up -d postgres
$env:HARNESS_DATABASE_URL = "postgres://harness:harness@127.0.0.1:5432/harness"
cargo run -p harness-cli -- migrate
```

If PostgreSQL is installed another way, set `HARNESS_DATABASE_URL` to that database and run the same migration script.

For WSL/k3s development, use `k8s/dev/postgres.yaml` and port-forward the service as documented in `docs/development/postgresql.md`.

More detail is in `docs/development/postgresql.md` and `docs/development/checks.md`.
