# Coding Agent Harness

Rust-first coding-agent harness runtime. The V0 goal is a PostgreSQL-backed runtime that records EventLog entries, policy decisions, tool observations, token usage, and recovery attempts while driving a CLI-started self-recovering coding task.

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

## PostgreSQL Development Baseline

Copy `.env.example` to `.env` and start a local PostgreSQL instance. If Docker Compose is available:

```powershell
docker compose up -d postgres
$env:HARNESS_DATABASE_URL = "postgres://harness:harness@127.0.0.1:5432/harness"
.\scripts\apply-migrations.ps1
```

If PostgreSQL is installed another way, set `HARNESS_DATABASE_URL` to that database and run the same migration script.

More detail is in `docs/development/postgresql.md` and `docs/development/checks.md`.
