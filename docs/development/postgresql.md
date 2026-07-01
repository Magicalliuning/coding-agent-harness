# PostgreSQL Development Baseline

V0 uses PostgreSQL as the primary runtime source of truth. Local development needs a PostgreSQL database reachable through `HARNESS_DATABASE_URL`.

## Option A: Docker Compose

```powershell
docker compose up -d postgres
$env:HARNESS_DATABASE_URL = "postgres://harness:harness@127.0.0.1:5432/harness"
.\scripts\apply-migrations.ps1
```

## Option B: Existing PostgreSQL

Create a database and user with equivalent permissions, then set:

```powershell
$env:HARNESS_DATABASE_URL = "postgres://USER:PASSWORD@HOST:PORT/DATABASE"
.\scripts\apply-migrations.ps1
```

## Migration Convention

Migrations live in `migrations/` and are applied in filename order by `scripts/apply-migrations.ps1`.

The script requires `psql` on `PATH`. It does not create the database; it only applies migrations to the database named by `HARNESS_DATABASE_URL`.
