# Development Checks

Run these commands before sending changes for review:

```powershell
cargo fmt --all --check
cargo test --workspace
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The V0 CLI bootstrap exposes two smoke commands:

```powershell
cargo run -p harness-cli -- --version
cargo run -p harness-cli -- doctor
```

With `HARNESS_TEST_DATABASE_URL` set to a reachable PostgreSQL database, run:

```powershell
cargo test -p harness-runtime --test postgres_session
cargo test -p harness-cli --test session_cli
```

These integration tests verify session creation, EventLog append, EventLog replay, and derived CLI session state through PostgreSQL.
