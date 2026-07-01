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

These commands intentionally cover only the bootstrap baseline. Later issues will add runtime-specific manual QA gates.
