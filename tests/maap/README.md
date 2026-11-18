# TelemetryDeck adapter tests

Run `node --test tests/maap/*.test.mjs` (Node 22.23.4+ with native TypeScript stripping). No package installation, database, or network required.

**`representative-swift-batch.json` is a synthetic, hand-authored example, not a debug capture.**
It includes a subset of the properties emitted by the [unmodified SwiftSDK 2.14.2](https://github.com/TelemetryDeck/SwiftSDK/tree/58f436299d3f6710bcedc18aff26480aaf1879fc): see `Signals/Signal.swift` for default properties and aliases, `Signals/SignalManager.swift` for serialization and whole-batch retries, `Helpers/JSONFormatting.swift` for date encoding, and `Presets/TelemetryDeck+Navigation.swift` for navigation keys (all under `Sources/TelemetryDeck`). Its IDs and events are made up.

The batch tests exercise the mapper and Request/Response boundary with a substitute for Umami's `POST`. `accepted` counts successful calls to `/api/send`, `rejected` counts invalid signals and failed calls, and `dropped` counts test signals when `TD_TEST_WEBSITE_ID` is unset. A parsed array is acknowledged even if a write fails; retrying the whole batch would duplicate successful siblings. These tests are not a database or iOS end-to-end run. The separate image smoke test exercises the real collector, both SDK trailing-slash paths, and persisted metrics.

The adapter expects Swift's flat `[String: String]` payload and hex SHA‑256 `clientUser` when present. It adds no batch-size limit (the pinned SDK pops at most 100 signals at once). Umami's existing website, timestamp and field validation/truncation still apply, including its 50-character stored distinct ID. `TD_ALLOWED_KEYS` applies to nested event-data keys; the top-level numerical `floatValue` and first-class dimensions are independent. If unset, all non-`TelemetryDeck.*` keys—including deprecated SDK aliases such as `region` and `locale`—are retained alongside the four selected SDK keys. Use an explicit allowlist to restrict them.
