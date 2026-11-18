# Send iOS signals to Maap

Use the **unmodified TelemetryDeck Swift SDK, exact version 2.14.2**. The contract below was checked against release source. No app was run here; the device-to-dashboard check is pending.

## 1. Connect the app

1. Follow the [operator runbook](../deploy/maap/README.md) to create **app and test websites**. Use the app website UUID as SDK `appID` in all builds. Set server `TD_TEST_WEBSITE_ID` to the **test** UUID and apply the setting. Maap routes Debug test signals there, or drops them if unset.
2. Obtain the operator's reachable **HTTPS ingest base URL** (e.g. `https://analytics.example.org/`), not the protected dashboard URL, `TD_HOSTNAME`, `/api/send` or a full `/v2/` endpoint. **Next may return 308** from `/v2/` to `/v2` by default: both POST paths must be reachable through Access without an interactive login, while the dashboard stays protected. Verify a final JSON response with the POST body preserved. Do not embed Access credentials.
3. In Xcode → **File → Add Package Dependencies**, enter `https://github.com/TelemetryDeck/SwiftSDK.git`, choose **Exact Version → 2.14.2** and add library **TelemetryDeck** to the app target. Verify `Package.resolved` revision `58f436299d3f6710bcedc18aff26480aaf1879fc`. The [package][package] requires Swift tools 5.9 and iOS 12+.
4. In the target's Info.plist (Target → Info), set String entries `MaapWebsiteID` and `MaapIngestBaseURL` locally. These are routing values, not secrets. Never paste passwords, keys, tokens or raw device logs into chat.
5. Call `startMaap()` **once** in the existing `App.init()` (or UIKit launch delegate), before tracking:

   ```swift
   import Foundation
   import TelemetryDeck

   // Call from the app entry point, not from a view's body.
   func startMaap() {
       let info = Bundle.main
       guard let id = info.object(forInfoDictionaryKey: "MaapWebsiteID") as? String,
             UUID(uuidString: id) != nil,
             let raw = info.object(forInfoDictionaryKey: "MaapIngestBaseURL") as? String,
             let base = URL(string: raw), base.scheme == "https", base.host != nil,
             base.user == nil, base.password == nil, base.query == nil, base.fragment == nil else {
           preconditionFailure("Set MaapWebsiteID and MaapIngestBaseURL for this test target")
       }
       let config = TelemetryDeck.Config(appID: id, baseURL: base)
       #if DEBUG
       config.testMode = true
       #else
       config.testMode = Bundle.main.appStoreReceiptURL?.lastPathComponent == "sandboxReceipt"
       #endif
       // Optional: insert the DEBUG-only capture handler below here, before initialize.
       TelemetryDeck.initialize(config: config)
   }
   ```

   The [Config][api] [initializer][config] is `init(appID: String, salt: String? = nil, baseURL: URL? = nil, namespace: String? = nil)`. `apiBaseURL` is read-only. **Don't omit `baseURL`**: `nil` selects `nom.telemetrydeck.com`. Leave `namespace` unset.

## 2. Track screens and an action

Call the [navigation API][navigation] once when **each** destination appears, or use the [SwiftUI view modifier][modifier], which does that for you. Don't do both. Use fixed screen names, never user IDs, filenames, search terms or URLs with query strings. In an existing SwiftUI app this can look like:

```swift
import SwiftUI
import TelemetryDeck

struct TestTabs: View {
    var body: some View {
        TabView {
            Text("Home")
                .trackNavigation(path: "/home")
                .tabItem { Text("Home") }
            Text("Library")
                .trackNavigation(path: "/library")
                .tabItem { Text("Library") }
            Button("Run test action") {
                TelemetryDeck.signal("MAAP.acceptanceTapped", parameters: ["surface": "settings"])
            }
            .trackNavigation(path: "/settings")
            .tabItem { Text("Settings") }
        }
    }
}
```

In UIKit call `TelemetryDeck.navigationPathChanged(to: "/home")` in `viewDidAppear(_:)`, substituting the current route. The previous destination becomes the next source (`""` initially). `navigate(to:)` is unavailable. Maap records navigation as pageviews and ordinary signals as named events.

## What actually crosses the wire

The [sender][send] preserves the base path, adds a slash **only if missing**, then appends `v2/`. Both `https://analytics.example.org` and `https://analytics.example.org/` produce **`POST https://analytics.example.org/v2/`**. A base ending in `/v2/` incorrectly yields `/v2/v2/`. Only a nonempty namespace selects `v2/namespace/<namespace>/` ([URL tests][url-tests]).

It sends `Content-Type: application/json`, no SDK-added authorization header, and a **top-level JSON array** (even for one signal), not a wrapper or one POST per event. Each element uses these **case-sensitive** keys from the [constructed body][body], [Codable model][model] and [encoder][dates]:

| Key | JSON value from 2.14.2 |
|---|---|
| `appID` | String copied from config. Not `appId`, `websiteId` or the namespace. |
| `clientUser`, `sessionID` | Strings: [lowercase SHA-256 hex][hash] of user identifier plus client salt (empty salt if omitted); SDK session UUID string. These are pseudonymous, not credentials. |
| `type` | Signal name, e.g. `MAAP.acceptanceTapped` or **`TelemetryDeck.Navigation.pathChanged`**. The navigation implementation emits the latter despite a stale doc comment mentioning `TelemetryDeck.Route.Transition.navigation`. |
| `receivedAt` | UTC date string, **`yyyy-MM-dd'T'HH:mm:ssZ`** (e.g. `2026-01-02T03:04:05+0000`, not a Unix number). It records the device's signal time, before the queued batch is sent. |
| `isTestMode` | **String** `"true"` / `"false"`, not JSON `true` / `false`. Defaults to the SDK's `DEBUG` setting; it does not prevent transmission. |
| `floatValue` | Optional JSON number; omitted when `nil`. |
| `payload` | **Nested object of string → string**, including SDK/device defaults and custom parameters. Not `parameters`, not `["key:value"]`, not top-level dimensions. For a navigation, look inside this object for **`TelemetryDeck.Navigation.destinationPath`**, `TelemetryDeck.Navigation.sourcePath`, `TelemetryDeck.Navigation.identifier` (`source -> destination`) and `TelemetryDeck.Navigation.schemaVersion` (`"1"`). |

`SignalPostBody` has a historical comment about V1. Follow `processSignal → encodeBatch → send → getServiceUrl` in [SignalManager.swift][send], not that comment: this very model is encoded as an array and sent to **v2**. The [default payload][defaults] can be large; it includes both legacy and namespaced names (e.g. `appVersion` and `TelemetryDeck.AppInfo.version`).

**Privacy:** A blank server `TD_ALLOWED_KEYS` is not a minimal-data setting: it retains unprefixed SDK aliases such as `region` and `locale` alongside custom keys. For less event data, have the operator choose an explicit comma-separated list of exact keys (e.g. `surface`); it replaces the default list. This filters stored event properties, not the SDK's outgoing body or Maap's Browser dimension.

## 3. Capture a real batch privately (Debug builds only)

Use Xcode **Run**, not Preview. Before `initialize`, set `config.logHandler = .standard(.debug)` inside `#if DEBUG` ([default is `.info`][logging]). Look for the **bare JSON array** from `send` in Xcode's console, not `Process signal: SignalPostBody(...)`. On modern iOS the [standard handler][handler] uses OSLog subsystem `TelemetryDeck`, category `LogHandler`.

If OSLog hides values or truncates long messages, substitute this public handler **instead**, locally on a clean test installation. It prints only encoded JSON arrays, not the SDK's other debug descriptions:

```swift
#if DEBUG
config.logHandler = LogHandler(logLevel: .debug) { level, message in
    guard level == .debug, message.first == "[" else { return }
    print("MAAP_TEST_BODY " + message)
}
#endif
```

Copy one complete `[...]` after the marker to a private file outside the repository. Check `jq -e 'type == "array" and length > 0 and all(.[]; (.payload | type) == "object")' private-batch.json`. If truncated, inspect `String(data: body, encoding: .utf8)` at `urlRequest.httpBody = body` in Xcode; don't reconstruct missing text. Remove breakpoints before timing.

**Fixture provenance:** [`tests/maap/representative-swift-batch.json`](../tests/maap/representative-swift-batch.json) is representative, source-derived input, **not a captured SDK log**. A genuine capture is pending. To share one, make a redacted **copy**: substitute same-shape website/session IDs and client hashes; scrub timestamps and device, locale, accessibility, user and custom values. Preserve observed keys, types, nesting and array boundaries. Record pin, Debug/device or simulator, capture method and redactions. Use the approved artifact channel; remove the handler and delete private captures after verification.

## Phase 5 manual acceptance: all four results within 60 seconds

| When | Developer action | Evidence |
|---|---|---|
| Before timing | Open **Realtime** for `TD_TEST_WEBSITE_ID` (`/websites/<testWebsiteId>/realtime`). Use a clean Debug install and synchronized clock. | Note the baseline so old activity won't count. |
| **T=0** | Start timing when the first tracked screen appears. Visit **Home → Library → Settings** and tap **Run test action** once. | Three appearances, one tap, one foreground session. |
| By T=60 s | Keep the app open; refresh Realtime if needed ([SDK interval][interval]: 10 s by default). | All three **fresh pageviews** `/home`, `/library`, `/settings` and **named event** `MAAP.acceptanceTapped` appear. Automatic session events don't count. |
| At completion | Record elapsed time, site alias, build, observed paths and event. | Mark timing FAIL if late or missing. A 2xx, pre-upload log, synthetic replay or totals alone do not pass. |

**Browser filter check:** On the test site's dashboard for this time range, verify **Browser** shows **`App <version>`** using the app's `TelemetryDeck.AppInfo.version`, not SDK version `2.14.2`. Apply **Browser = App \<version\>**; **Pages** must still show all three screen paths and **Events** the custom name. Record the filter value and results. Confirm the Debug run did not appear on the production site.

No data? Check consent/opt-out, initialization, batch, actual HTTPS POST path, JSON response/accepted count and test website in that order. The [SDK treats any 2xx as transport success][response], drops some rejected batches (e.g. 400/404/413/422) and retries transient failures. After fixing the cause, start a **new** timed run.

[package]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Package.swift#L1-L17
[api]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/TelemetryDeck.swift#L3-L46
[config]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/TelemetryClient.swift#L188-L209
[send]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/Signals/SignalManager.swift#L339-L389
[url-tests]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Tests/TelemetryDeckTests/TelemetryClientTests.swift#L15-L69
[body]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/Signals/SignalManager.swift#L152-L209
[model]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/Signals/Signal.swift#L14-L40
[defaults]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/Signals/Signal.swift#L42-L133
[dates]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/Helpers/JSONFormatting.swift#L48-L57
[hash]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/Helpers/CryptoHashing.swift#L9-L30
[navigation]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/Presets/TelemetryDeck%2BNavigation.swift#L23-L77
[modifier]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/Modifiers/TrackNavigationModifier.swift#L4-L38
[logging]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/TelemetryClient.swift#L131-L180
[handler]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/Helpers/LogHandler.swift#L7-L67
[interval]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/TelemetryClient.swift#L93-L127
[response]: https://github.com/TelemetryDeck/SwiftSDK/blob/58f436299d3f6710bcedc18aff26480aaf1879fc/Sources/TelemetryDeck/Signals/SignalManager.swift#L436-L455
