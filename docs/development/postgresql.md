# PostgreSQL Development Baseline

V0 uses PostgreSQL as the primary runtime source of truth. Local development needs a PostgreSQL database reachable through `HARNESS_DATABASE_URL`.

## Option A: Docker Compose

```powershell
docker compose up -d postgres
$env:HARNESS_DATABASE_URL = "postgres://harness:harness@127.0.0.1:5432/harness"
cargo run -p harness-cli -- migrate
```

## Option B: Existing PostgreSQL

Create a database and user with equivalent permissions, then set:

```powershell
$env:HARNESS_DATABASE_URL = "postgres://USER:PASSWORD@HOST:PORT/DATABASE"
cargo run -p harness-cli -- migrate
```

## Option C: k3s on WSL

For the local WSL/k3s development cluster:

```powershell
wsl.exe sh -lc "kubectl apply -f /mnt/c/AI/Projects/基座/k8s/dev/postgres.yaml"
wsl.exe sh -lc "kubectl -n coding-agent-harness-dev rollout status deploy/postgres"
wsl.exe sh -lc "kubectl -n coding-agent-harness-dev port-forward svc/postgres 55432:5432 --address 0.0.0.0"
```

In another terminal:

```powershell
$wslIp = (wsl.exe hostname -I).Trim().Split(" ")[0]
$env:HARNESS_TEST_DATABASE_URL = "postgres://harness:harness@${wslIp}:55432/harness"
cargo test -p harness-runtime --test postgres_session
cargo test -p harness-cli --test session_cli
```

On some WSL setups, Windows can reach the forwarded port through `127.0.0.1`. If that fails, use the WSL IP as shown above.

## Migration Convention

Migrations live in `migrations/` and are applied in filename order by `scripts/apply-migrations.ps1`.

The script requires `psql` on `PATH`. It does not create the database; it only applies migrations to the database named by `HARNESS_DATABASE_URL`.

The CLI `migrate` command applies the same embedded migration without requiring `psql`.
