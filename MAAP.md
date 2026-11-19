# Maap (माप) — Mobile App Analytics Platform

*Maap* means **measure** in Hindi (माप) and stands for **Mobile App Analytics Platform**.

Maap adds a TelemetryDeck-compatible receiver and multi-platform app support (`iOS`, `macOS`, `Android`, and `Website`) to Umami 3.5.0. An unmodified mobile/desktop SDK posts its
batch to `/v2/` or `/v2/namespace/<name>/`; each signal goes through the
existing `/api/send` handler in-process. No translation service is required.

| From the SDK | In the dashboard |
| --- | --- |
| `TelemetryDeck.Navigation.pathChanged` | Pageview at the destination screen path |
| Any other signal type | Named event at `/` |
| `TelemetryDeck.AppInfo.version` | Browser `App <version>` |
| `isTestMode: "true"` | Tagged `tag = "test"` on the same website; filtered by the dashboard **Test Mode** toggle |

`appID` is the production website UUID. `clientUser` is already hashed; Umami
derives its own sessions and visits rather than importing `sessionID`. Signals
older than five minutes use their SDK timestamp; fresh signals use Umami's live
session clock. A parsed batch is acknowledged even if individual signals fail.
Inspect the counts, not just HTTP 200.

**Build and release.** The **Maap image** GitHub Actions workflow runs the adapter
tests on Linux, builds the upstream Dockerfile for `linux/amd64`, and exercises
the image against disposable Postgres before publishing. Pull requests do not
publish. Successful default-branch runs publish `ghcr.io/yashasg/umami-td`
tagged with the full Git SHA and `latest`, then deploy that exact SHA to the
GCP `e2-micro` VM (`maap-vm` in `us-west1-b`) over IAP SSH using keyless GCP
Workload Identity Federation (`deploy/maap/deploy.sh`).

- [Operator runbook](deploy/maap/README.md): use the **Maap** Compose file (not
  the upstream root Compose file), enroll TOTP, set up websites and MCP, and
  verify backups. Supply an already-protected dashboard and public ingestion paths.
- [iOS integration](docs/maap-ios.md): exact Swift SDK pin and initializer,
  redacted capture procedure, and the timed three-screen/one-event acceptance check.

## Server Runtime (`MAAP_SERVER`)

- **`MAAP_SERVER=rust` (default):** Rust serves the API and dashboard on `:3000` with `MAAP_NODE_FALLBACK=0`. Node runs the startup checks and migrations, then exits; no Next.js server stays running. Reports can combine a saved cohort with event and session property filters. The cohort's own date range selects visitors; the report's date range selects their activity.
- **`MAAP_SERVER=node`:** Explicit rollback to the reference Next.js server. Use this for `CLICKHOUSE_URL`, `REDIS_URL`, or `CLOUD_MODE`; Rust refuses to start with those settings. `MAAP_NODE_FALLBACK=1` is not a rollback: it starts Next.js only for routes Rust does not handle. The release workflow tests both runtimes against disposable Postgres.
