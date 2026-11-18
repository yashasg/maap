# Maap: iOS telemetry for Umami

Maap adds a TelemetryDeck receiver to Umami 3.5.0. An unmodified iOS SDK posts its
batch to `/v2/` or `/v2/namespace/<name>/`; each signal goes through Umami's
existing `/api/send` handler in-process. No translation service is required.

| From the SDK | In the dashboard |
| --- | --- |
| `TelemetryDeck.Navigation.pathChanged` | Pageview at the destination screen path |
| Any other signal type | Named event at `/` |
| `TelemetryDeck.AppInfo.version` | Browser `App <version>` |
| `isTestMode: "true"` | Separate `TD_TEST_WEBSITE_ID`, or intentionally dropped if unset |

`appID` is the production website UUID. `clientUser` is already hashed; Umami
derives its own sessions and visits rather than importing `sessionID`. Signals
older than five minutes use their SDK timestamp; fresh signals use Umami's live
session clock. A parsed batch is acknowledged even if individual signals fail.
Inspect the counts, not just HTTP 200.

**Build and release.** The **Maap image** GitHub Actions workflow runs the adapter
tests on Linux, builds the upstream Dockerfile for `linux/amd64`, and exercises
the image against disposable Postgres before publishing. Pull requests do not
publish. Successful default-branch runs publish `ghcr.io/yashasg/umami-td`
tagged with the full Git SHA and `latest`. Deploy the SHA, not `latest`. The
fork needs Actions enabled and permission for its workflow to write GHCR packages.

- [Operator runbook](deploy/maap/README.md): use the **Maap** Compose file (not
  the upstream root Compose file), enroll TOTP, set up websites and MCP, and
  verify backups. Supply an already-protected dashboard and public ingestion paths.
- [iOS integration](docs/maap-ios.md): exact Swift SDK pin and initializer,
  redacted capture procedure, and the timed three-screen/one-event acceptance check.
