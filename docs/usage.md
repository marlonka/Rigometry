# User guide

[Back to Rigometry](../README.md)

## Navigation

| Action | Shortcut |
| --- | --- |
| Overview, CPU & Memory, GPU, Storage | `Ctrl+1` through `Ctrl+4` |
| Rescan the current storage path | `F5` |
| Cancel a storage scan | `Esc` |
| Parent storage folder | `Alt+Up` |

Settings includes dark/light/system themes, scale from 85–150% and reduced motion. At smaller window sizes, sections stack and the page scrolls vertically.

## Language

Choose **Settings → Language**: English, Deutsch, Français, Español or System language. Switching applies immediately and is saved for the next launch. System language follows the Windows display language, including regional variants; other languages fall back to English.

Labels, dialogs opened by the app, status messages, diagnostics and command-line help use the selected language. Native Windows dialog controls and operating-system or driver error text follow their source language. Hardware names, paths, identifiers and standard unit symbols remain unchanged. Readings use decimal commas in German, French and Spanish.

Use `--language en`, `--language de`, `--language fr`, `--language es` or `--language system` to choose a language at startup. For example, `Rigometry.exe --language de --help` displays German help. Export field names, availability codes, raw provider messages and numeric data remain language-independent so existing automation continues to work.

## Units and accounting

Byte values use binary units consistently:

| Unit | Bytes |
| --- | ---: |
| KiB — kibibyte | 1,024 |
| MiB — mebibyte | 1,048,576 |
| GiB — gibibyte | 1,073,741,824 |
| TiB — tebibyte | 1,099,511,627,776 |

Decimal kB/MB/GB/TB use powers of 1000 instead. For example, 32 GiB is approximately 34.36 GB; changing only the suffix would mislabel the value. Uppercase **B** means bytes; lowercase **b** means bits. These are the standard [binary prefixes documented by NIST](https://physics.nist.gov/cuu/Units/binary.html).

Installed RAM is firmware capacity. Memory available to Windows may be smaller; memory in use is a live operating-system reading. These are separate measurements.

Storage **Logical** counts file lengths, named streams and each hard-link path. **Allocated** deduplicates file identities across the scan, attributing physical storage to the first encountered path. Directory indexes, filesystem metadata and other volume bookkeeping are excluded. Drive usage and scan allocation answer different questions.

`≥` marks a known lower bound. Cancellation, inaccessible entries or a metadata limit can leave totals incomplete. Expand scan accounting to inspect exclusions and errors.

## Sensors

The app records a 120-second window. Minimum and maximum values use that recorded history. Missing samples remain gaps; a reading becomes stale after five seconds without an update. Firmware specifications and live measurements are labeled separately.

Choose the GPU adapter before comparing readings. Device-wide usage/capacity and process-local memory usage/budgets are different fields. See [compatibility and sensor limits](../README.md#compatibility).

## Command line

Use `--help` for options and `--version` for the build version. Output parent directories must exist; output files must have new names. Existing files and aliases are not intentionally replaced. `.csv` selects CSV regardless of case; other export extensions select JSON.

```powershell
New-Item -ItemType Directory -Force .\artifacts | Out-Null

# Inventory and one measured sample after a baseline interval.
.\target\release\rigometry.exe --headless --report .\artifacts\hardware.json

# Folder scan with an explicit output file.
.\target\release\rigometry.exe --headless --scan 'C:\Data' --export .\artifacts\scan.json
.\target\release\rigometry.exe --headless --scan 'C:\Data' --export .\artifacts\scan.csv

# Open the desktop app and begin scanning.
.\target\release\rigometry.exe --scan 'C:\Data'
```

Exports contain paths and hardware details. Review before sharing. Concurrent filesystem changes are possible; scans are observations, not atomic snapshots. Cancellation cannot interrupt every blocked Windows API call.

## Screenshots

Capture mode renders five application views, then closes. It requires a Windows graphics session and new output names:

```powershell
.\target\release\rigometry.exe --capture .\artifacts\screenshots --scan .\src
.\target\release\rigometry.exe --capture .\artifacts\screenshots-light --theme Light --scale 1.5
.\target\release\rigometry.exe --capture .\artifacts\screenshots-small --window-size 1080x720 --scale 1.5
```

Capture output is evidence for manual inspection; successful file creation does not establish visual quality or accessibility.
