param(
    [string]$DatabaseUrl = $env:HARNESS_DATABASE_URL,
    [string]$MigrationsPath = "migrations"
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($DatabaseUrl)) {
    throw "HARNESS_DATABASE_URL is not set. See docs/development/postgresql.md."
}

if (-not (Get-Command psql -ErrorAction SilentlyContinue)) {
    throw "psql was not found on PATH. Install PostgreSQL client tools before applying migrations."
}

if (-not (Test-Path -LiteralPath $MigrationsPath)) {
    throw "Migrations path '$MigrationsPath' does not exist."
}

$migrationFiles = Get-ChildItem -LiteralPath $MigrationsPath -Filter "*.sql" | Sort-Object Name

if ($migrationFiles.Count -eq 0) {
    Write-Host "No migrations found in $MigrationsPath."
    exit 0
}

foreach ($migration in $migrationFiles) {
    Write-Host "Applying migration $($migration.Name)"
    & psql $DatabaseUrl -v ON_ERROR_STOP=1 -f $migration.FullName
}
