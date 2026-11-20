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

## App reports

The interface uses [Satoshi by Indian Type Foundry](https://www.fontshare.com/fonts/satoshi). Maap serves the font locally: the normal development, source build, or container startup fetches the unmodified variable WOFF2 once from Fontshare under its [Free Font License](https://www.fontshare.com/licenses/ffl). No browser request goes to a font provider. For an offline install, run `pnpm font:setup` while online and retain `public/fonts/satoshi-variable.woff2` on that instance. Without it, the interface falls back to the system font.

Start at `/websites` (or `/teams/<teamID>/websites`) to see the workspace’s apps and websites. Card counts follow the selected date range and Test Mode. Folders organize properties within the same workspace; moving an app does not change its ID or telemetry. Migration `30_workspace_folders` adds the folder and assignment tables without rewriting existing events.

Activation is private to accounts with access to the app; an Overview share does not include its event-property breakdowns. Removing an app from a shared board revokes that board’s access to its reports. View-only accounts can read notebooks and app settings but cannot change them.

Migration `31_notebook_soft_delete_cleanup` permanently removes notebooks belonging to already soft-deleted apps and does the same on future soft deletions. It does not change live apps’ notebooks or analytics events.

Open an iOS, macOS, or Android property at `/websites/<appID>` to see its Overview. The app name and summary sit above the report tabs; sections such as Metrics have their own sidebar. The website Traffic and Behavior menus do not appear in app reports. Report links are bookmarkable, for example `/websites/<appID>/app-reports/versions`, and retain the date range and Test Mode when navigating between reports. Regular website properties keep their existing analytics dashboard.

Versions uses metadata on **each event**, not the browser value saved when a session began. “Not reported” means the SDK did not send the dimension or the collector did not retain it; older events are not backfilled. Use an isolated app and an explicit allowlist to evaluate additional SDK dimensions before enabling them for users.

In **Dashboards**, choose **New Dashboard**, enter a name, and choose **Create**. The saved dashboard appears in the sidebar and has a URL you can reopen. **Design dashboard** opens the component editor. **Import Dashboard** accepts version 1 Maap Board JSON, not TelemetryDeck export files.

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
