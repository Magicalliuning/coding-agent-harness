# Local Codex CLI Manual Acceptance

This manual check validates the governed real Codex CLI worker lane without making Codex internal state authoritative.

## Prerequisites

- `codex` is installed and available on `PATH`, or pass `--codex-program <path>`.
- Codex auth is available through `CODEX_API_KEY` or `CODEX_HOME/auth.json`.
- PostgreSQL is available through `HARNESS_DATABASE_URL` or `--database-url`.
- The target repo is a Git repository.

Codex non-interactive mode is run with `codex exec`, which is the documented scripting entrypoint. The harness uses `--sandbox workspace-write` so Codex can write only inside the allocated Task Worktree. See the OpenAI Codex CLI non-interactive mode docs: https://developers.openai.com/codex/noninteractive.

## Run

```powershell
cargo run -p harness-cli -- migrate --database-url $env:HARNESS_DATABASE_URL

$start = cargo run -p harness-cli -- session start `
  --repo . `
  --database-url $env:HARNESS_DATABASE_URL

# Use the printed session_id value.
cargo run -p harness-cli -- session codex-acceptance <SESSION_ID> `
  --database-url $env:HARNESS_DATABASE_URL
```

Expected real-run signals:

- `codex_acceptance_status=ran`
- `codex_available=true`
- `codex_authenticated=true`
- `worker_final_status=succeeded` when Codex exits successfully
- `worker_pending_commit_state=pending_commit_approval` when Codex leaves a reviewable diff
- `event_replay_last=commit.approval_pending` for a successful diff-producing run

The harness must not commit Codex changes. Review and hand off the Task Worktree diff separately.

## Queue-Driven Worker Lane

The queue-driven path validates that PostgreSQL is the Task source of truth: create a Task, enqueue it, lease one queued Task, then run the governed Codex lane for that lease.

```powershell
$create = cargo run -p harness-cli -- session task create <SESSION_ID> `
  --database-url $env:HARNESS_DATABASE_URL `
  --repo . `
  --task "Create or update .harness/codex-queued-acceptance.md with a short note saying queued Codex acceptance ran successfully. Do not commit the change."

$taskId = ($create | Select-String '^task_id=').Line.Split('=')[1]

cargo run -p harness-cli -- session task enqueue <SESSION_ID> $taskId `
  --database-url $env:HARNESS_DATABASE_URL

cargo run -p harness-cli -- session task run-next-codex-worker `
  --database-url $env:HARNESS_DATABASE_URL `
  --worker-id local-codex-worker `
  --timeout-ms 300000 `
  -- powershell -NoProfile -Command "codex exec --sandbox workspace-write --json `$env:HARNESS_WORKER_TASK"
```

Expected successful queued-run signals:

- `task_leased=true`
- `worker_final_status=succeeded`
- `worker_pending_commit_state=pending_commit_approval` when Codex leaves a reviewable diff
- `task_status=pending_commit_approval`
- `task_queue_status=completed`
- `task_lease_worker_id=local-codex-worker`
- `task_lease_status=completed`
- `task_worker_status=succeeded`
- `task_approval_state=pending_commit_approval`

The command after `--` is the governed worker subprocess. It receives the queued Task input through `HARNESS_WORKER_TASK` and runs inside the allocated Task Worktree.

## Explicit Skip Path

If Codex is unavailable or auth is not detected, the command exits successfully with an explicit skip result instead of pretending acceptance passed:

```text
codex_acceptance_status=skipped
codex_available=false
codex_authenticated=false
codex_skipped_reason=...
```

Common skipped reasons:

- the Codex executable cannot be started
- `codex --version` exits non-zero
- no `CODEX_API_KEY` and no `CODEX_HOME/auth.json`

Automated tests cover fake/subprocess lanes and explicit skip behavior only. They do not require a real Codex login or executable.
