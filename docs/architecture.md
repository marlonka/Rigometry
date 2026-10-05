# Rigometry architecture

Rust 2024, Windows 11 x64, eframe/egui 0.34.3 with Glow and AccessKit. `Cargo.lock` and exact direct dependency versions define the build. Windows APIs and an optionally installed NVIDIA runtime supply real data; there is no web frontend, account service, telemetry backend, or bundled hardware driver.

## Modules and ownership

| Module | Responsibility |
| --- | --- |
| `main.rs` | Native window and CLI entry points; headless hardware report/scan/export; optional native screenshot capture |
| `model.rs` | Hardware inventory and timestamped readings; explicit availability states and binary byte formatting |
| `hardware.rs` | CPUID inventory and provider composition |
| `hardware/smbios.rs` | Bounded firmware-table parser; board/BIOS/module records and sentinel handling |
| `hardware/windows.rs` | Native CPU times/memory, DXGI identities, WDDM counters and D3DKMT queries |
| `hardware/nvml.rs` | Runtime loading, exact PCI matching and individual NVIDIA sensor calls |
| `storage.rs` and `storage/*` | Read-only metadata traversal, MFT-assisted path, accounting, cancellation and exports |
| `state.rs` | Worker messages, bounded history, scan aggregation, navigation, filtering/sorting and background dialogs/exports |
| `ui.rs` | Native pages, virtual table, charts/treemap, copy/reveal actions, settings and screenshot handling |
| `i18n.rs` and `i18n/*.json` | Presentation translations, Windows language detection and locale-specific number formatting; exports remain language-independent |

## Work and memory bounds

The hardware worker discovers inventory once, then samples about once per second. A capacity-four channel bounds pending events. History retains at most 120 telemetry samples and removes samples older than 120 seconds; valid readings become stale after five seconds without an update. Worker failure is reported separately from a valid zero.

Rendering runs independently from acquisition. Live chart views request a repaint about every 33 milliseconds for timestamp-based panning; this neither samples hardware at 30 Hz nor creates extra readings. Reduced motion disables core-bar interpolation and uses a lower periodic repaint rate. Fixed numeric columns and reserved scrollbar space prevent routine data changes from shifting adjacent controls.

Each storage scan owns its worker, cancellation flag and receiver. Metadata nodes arrive in batches of up to 256 through a capacity-nine queue, reserving room for completion. The UI consumes scan events for up to eight milliseconds per update and throttles visible-index rebuilding to 400 milliseconds during active scans. A replacement scan gets a fresh receiver, preventing old batches from attaching to the new tree.

Nodes occupy a flat vector with parent/child IDs. Aggregate totals add owned bytes to ancestors exactly once. Table rows are virtualized; treemap layout is cached by revision and viewport. Completed scan exports share an `Arc` snapshot and serialize on a worker. The full scan still requires memory proportional to entries; filtering/sorting and aggregation can perform work proportional to the tree on the UI thread. No claim of unlimited-volume memory bounds or zero-latency million-entry filtering is made.

## Data contracts

- Hardware `Field` and `Reading` retain value, source, units, availability, detail and timestamp. Nonfinite sensor values are rejected. Firmware configuration, nominal specifications and live telemetry have distinct labels.
- Windows GPU providers join by DXGI LUID. NVML requires a unique PCI bus/device/function plus vendor/device match. Names and enumeration order never establish identity. Ambiguous matches fail explicitly.
- GPU adapter usage, process-local DXGI usage/budget and capacity are distinct. Driver-specific fields carry individual support/failure states.
- Logical storage bytes include named streams and each hard-link path. Allocated bytes count each file identity once across the scan, attributed to the first encountered path. Nested scopes preserve this attribution.
- File/directory named streams are included; directory indexes, MFT/security/journal/snapshot overhead are excluded. Volume used space and scan allocation therefore answer different questions.
- Reparse points and cloud placeholders are excluded; metadata handles request no recall and no following. Errors, cancellation and exclusions remain in summaries/exports. Incomplete totals are lower bounds, not exact totals.
- The MFT index supplies names/relationships only. Metadata enrichment and hard-link name enumeration are still required. Unsupported or denied volume access falls back to normal enumeration.

## Runtime and distribution

NVIDIA DLLs load only from known absolute installed-driver locations. No driver DLL is shipped. Other vendors use documented Windows interfaces; additional vendor SDKs are not integrated. Privileged CPU sensors and SPD/controller access remain absent.

System Segoe UI fonts are loaded from Windows when available, with installed MS Gothic for Japanese glyph fallback; these Windows fonts are not redistributed. Default egui fonts provide fallback and require their bundled notices. Portable distribution must include the application license, dependency inventory and third-party license text. See [dependency notices](dependency-notices.md).

Supported systems and physical test coverage are listed in the [README](../README.md#compatibility). Benchmarks should state build, dataset, cache state, hardware and timing definition; test success does not establish every device or full UI workflow.

The `egui_kittest` development dependency runs production UI journeys through semantic controls and keyboard events. Real temporary filesystem fixtures cover scan/drill-down/filter and cancellation/restart isolation. UI/state tests inject a hardware event source that never emits, then supply explicit inventory/reading fixtures where needed. This prevents asynchronous device discovery from moving pointer targets. Production still starts the native monitor; native provider tests and release verification cover that boundary. Process tests execute the actual CLI. Fixtures are never application demo data. See [development checks](development.md#checks).
