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
| `storage.rs` and `storage/*` | Read-only metadata traversal: parallel folder listings, NTFS master file table reader, accounting, cancellation and exports |
| `state.rs` | Worker messages, bounded history, scan aggregation, navigation, filtering/sorting and background dialogs/exports |
| `search.rs` | Storage search: query syntax, typo-tolerant ranking and a linear pass over the scan tree with per-scan name masks (adapted from FSearch, MIT) |
| `ui.rs` | Native pages, virtual table, charts/treemap, copy/reveal actions, settings and screenshot handling |
| `i18n.rs` and `i18n/*.json` | Presentation translations, Windows language detection and locale-specific number formatting; exports remain language-independent |

## Work and memory bounds

The hardware worker discovers inventory once, then samples about once per second. A capacity-four channel bounds pending events. History retains at most 120 telemetry samples and removes samples older than 120 seconds; valid readings become stale after five seconds without an update. Worker failure is reported separately from a valid zero.

Rendering runs independently from acquisition. Live chart views request a repaint about every 33 milliseconds for timestamp-based panning; this neither samples hardware at 30 Hz nor creates extra readings. Reduced motion disables core-bar interpolation and uses a lower periodic repaint rate. Fixed numeric columns and reserved scrollbar space prevent routine data changes from shifting adjacent controls.

Each storage scan owns its worker, cancellation flag and receiver. Metadata nodes arrive in batches of up to 256 through a capacity-nine queue, reserving room for completion. The UI consumes scan events for up to eight milliseconds per update and throttles visible-index rebuilding to 400 milliseconds during active scans; typing, sorting and expanding rebuild on the next frame. A replacement scan gets a fresh receiver, preventing old batches from attaching to the new tree.

Search makes one pass in ID order, where parents precede children, so folder words propagate without building paths; only folders keep per-word state. Each name's character-class mask is computed once per scan and extended as entries arrive, so most names are rejected without decoding. `path:` filters build each folder's path once and extend it per entry. On a real 3.2-million-entry drive (release build), queries take 22–121 ms at p50, after a one-time 67 ms mask build; see [benchmarks](benchmarks.md). Results and sorting still run on the UI thread.

Nodes occupy a flat vector with parent/child IDs. Aggregate totals add owned bytes to ancestors exactly once. Table rows are virtualized; treemap layout is cached by revision and viewport. Completed scan exports share an `Arc` snapshot and serialize on a worker. Nodes store names, not paths; full paths are joined from ancestors when needed. The full scan still requires memory proportional to entries: a 3.1-million-entry system drive peaked at about 1 GB. Scans stop at a 1.5 GiB metadata estimate, about 5 million entries, and report that first; filtering/sorting and aggregation can perform work proportional to the tree on the UI thread. No claim of unlimited-volume memory bounds or zero-latency million-entry filtering is made.

## Data contracts

- Hardware `Field` and `Reading` retain value, source, units, availability, detail and timestamp. Nonfinite sensor values are rejected. Firmware configuration, nominal specifications and live telemetry have distinct labels.
- Windows GPU providers join by DXGI LUID. NVML requires a unique PCI bus/device/function plus vendor/device match. Names and enumeration order never establish identity. Ambiguous matches fail explicitly.
- GPU adapter usage, process-local DXGI usage/budget and capacity are distinct. Driver-specific fields carry individual support/failure states.
- Logical storage bytes count each hard-link path. Allocated bytes count each file identity once across the scan, attributed to the first encountered path. Nested scopes preserve this attribution.
- Standard scans count each file's main data stream; administrator scans of whole drives also count named streams and restore points in System Volume Information. Directory indexes and MFT/security/journal overhead are always excluded. Volume used space and scan allocation therefore answer different questions.
- Standard scans read folder listings (`FileIdExtdDirectoryInfo`, falling back to older classes on FAT and network drives): one query returns name, sizes, attributes and file ID for many entries, so no file is opened. Eight threads list folders; the scan thread numbers entries and deduplicates hard links, so IDs stay append-only and parents precede children. Each folder is reopened by path and checked on its handle before listing, so a folder replaced by a junction is not followed. Listing records are parsed with bounds checks, and names containing separators or stream syntax are rejected.
- Reparse points and cloud placeholders are excluded; handles request no recall and no following. Errors, cancellation and exclusions remain in summaries/exports. Incomplete totals are lower bounds, not exact totals.
- Whole-drive scans with administrator rights read the NTFS master file table directly, unbuffered, in 4 MiB blocks that several threads read and parse. Names, folders and sizes come from file records; allocation is the sum of each data stream's non-sparse runs, so compressed, sparse and CompactOS files are exact, and no file is opened. Records whose fixups disagree were read mid-write and are read again. Every offset is bounds-checked; a volume the parser cannot handle, a non-NTFS drive or missing rights fall back to normal enumeration before any node is emitted. Folder scans always enumerate, since they would still read the whole table.

## Runtime and distribution

NVIDIA DLLs load only from known absolute installed-driver locations. No driver DLL is shipped. Other vendors use documented Windows interfaces; additional vendor SDKs are not integrated. Privileged CPU sensors and SPD/controller access remain absent.

System Segoe UI fonts are loaded from Windows when available, with installed MS Gothic for Japanese glyph fallback; these Windows fonts are not redistributed. Default egui fonts provide fallback and require their bundled notices. Portable distribution must include the application license, dependency inventory and third-party license text. See [dependency notices](dependency-notices.md).

Supported systems and physical test coverage are listed in the [README](../README.md#compatibility). Benchmarks should state build, dataset, cache state, hardware and timing definition; test success does not establish every device or full UI workflow.

The `egui_kittest` development dependency runs production UI journeys through semantic controls and keyboard events. Real temporary filesystem fixtures cover scan/drill-down/filter and cancellation/restart isolation. UI/state tests inject a hardware event source that never emits, then supply explicit inventory/reading fixtures where needed. This prevents asynchronous device discovery from moving pointer targets. Production still starts the native monitor; native provider tests and release verification cover that boundary. Process tests execute the actual CLI. Fixtures are never application demo data. See [development checks](development.md#checks).
