# Changelog

## Unreleased

- **Duplicate files:** a new **Duplicates** view on the Storage page compares the files below the current folder by content, after a scan and only on request. Only same-size files are read; the first and last 16 KiB rule out most, and BLAKE3 hashes confirm the rest. The biggest savings appear first, the oldest copy leads each group, and the search filters groups. Hard links are not copies, APFS clones count only their own blocks, cloud files are never downloaded and changed files are left out. Export as CSV or JSON, or run `--headless --scan PATH --duplicates FILE [--min-size SIZE]`. [How it works](docs/usage.md#duplicate-files)
- Scans in the window are as fast as headless scans: results no longer wait for the next repaint, which made standard scans of large drives about ten times slower.
- Sorting large result lists by name and searching with filters only (`kind:file`, `ext:jpg`) no longer take seconds per keystroke on millions of entries.
- A context menu acts on the entry you right-clicked even when rows re-sort under it during a scan.
- Windows: folders whose names end in a dot or space (`foo.`) are scanned themselves instead of failing or counting a sibling twice; empty FAT/exFAT drives scan without an error.
- Repeated search filters narrow each other (`size:>1g size:<4g`), quoted phrases match as written, and `.gitignore` finds `.gitignore` first.
- Headless commands check every output before working and fail with exit code 1 when the scan folder cannot be read; `--scale` and `--window-size` reject values they used to clamp.
- Escape leaves text fields and menus before it cancels a scan; Ctrl/⌘ + plus no longer zooms past the scale setting; the scale slider no longer flickers while dragged.
- GPU readings: an unplugged eGPU on a Mac no longer shows the integrated GPU's load; Windows sensors a driver does not have are unsupported instead of 0; the first CPU sample no longer reports every core as failed.
- macOS shows ⌘ shortcuts, its own platform label and the shared memory manufacturer; Korean file names render; system tools that hang are stopped after 30 seconds.
- Sizes just below a unit read 1.00 MiB instead of 1024.00 KiB; file paths in messages are never translated; translation fixes in French and Spanish.

## 0.3.0

- **Rigometry for Mac:** one universal app for Apple silicon and Intel Macs, macOS 11 or later. [Install guide](docs/usage.md#install-on-macos)
- On a Mac, the CPU page shows the chip, performance and efficiency cores, per-cluster caches and Arm instruction-set features; Intel Macs keep the full CPUID details.
- The GPU page lists every Mac graphics processor with cores, Metal support, video memory and connected displays, and charts live utilization from the graphics driver. Dual-GPU Intel Macs also show temperature, clocks, fan and power where the driver publishes them.
- Mac model, firmware version, installed memory, memory type and manufacturer appear in Overview and Diagnostics. Serial numbers and hardware UUIDs are never read into reports.
- Storage scans on a Mac read whole folder listings on up to eight threads with `getattrlistbulk`, without opening files, and search works exactly as on Windows. Scanning `/` counts the startup disk once and does not enter other mounted volumes; files stored only in iCloud are not downloaded. Approach adapted from [FSearch](https://github.com/noahdunnagan/fsearch) (MIT).
- **Show in Finder**, ⌘1–⌘4 shortcuts and the macOS language setting replace their Windows counterparts on a Mac.
- CI builds, tests and packages the Mac app on every push and attaches it to tagged releases.

## 0.2.0

- Storage search replaces the plain filter: ranked, typo-tolerant name matching that also uses folder names, with `'exact`, `^prefix`, `suffix$`, `!exclude` and `ext:`, `type:`, `kind:`, `size:`, `path:` filters. Adapted from [FSearch](https://github.com/noahdunnagan/fsearch) (MIT).
- Storage shows one size, bytes on disk, across table, map, totals and search; file size remains in details, tooltips and exports.
- Standard scans read whole folder listings on eight threads instead of opening every file: `C:\` with 3.1 million entries in about 10 s instead of 8 min, faster than `robocopy`. Files Windows keeps locked (`pagefile.sys`, `hiberfil.sys`) now show their size instead of "Unavailable". Named data streams count only in administrator drive scans.
- Right-click rows and map tiles to show them in Explorer, copy the path or open a folder; each row also has an Explorer button.
- Searching, sorting and expanding respond immediately during a scan, and the search no longer makes the table jump.
- Whole-drive scans no longer stop partway through. Entries no longer store their full path, the limit is 1.5 GiB, and a scan that still reaches it says so first. Before, a typical `C:\` stopped after `Users` and `Windows` without saying why.
- Folders Windows protects count as needing administrator rights, not as errors. **Scan as administrator** scans them after Windows asks for consent.
- The size map takes the space the name column doesn't need and appears beside the table in narrower windows.
- Administrator scans of whole NTFS drives read the master file table directly: about 3 s for `C:\`, no file opened, including protected folders, restore points and named streams.
- The file count appears only for folders, and the largest-files list drops the column.
- Typing a bare drive letter such as `C:` scans the drive root, not the folder Windows remembers as current on that drive; drive-relative paths like `C:Users` are refused with a hint.
- Paths pasted with quotes, as Explorer's "Copy as path" adds them, are accepted.
- Command-line output such as `--help` and `--version` appears in the terminal that started the release build.
- [Benchmarks](docs/benchmarks.md) for whole-drive scans and search, with the commands to reproduce them.

## 0.1.0

Initial Rigometry release for Windows 11 on Intel and AMD x64 PCs.

- Native Windows CPU, memory and GPU inspection with live history and explicit sensor availability.
- Read-only folder and drive analysis with a hierarchy, size map, filtering and CSV/JSON export.
- English, German, French and Spanish interfaces, saved language selection and localized readings.
- Monochrome interface with scalable icons, dark/light themes and reduced motion.
- Protected output creation, bounded workers and automated filesystem, UI and command-line checks.
- Documented platform support and a verified Windows x64 release pipeline.

Compatibility and sensor limits are documented in the [README](README.md#compatibility). Downloads are available in [Releases](https://github.com/marlonka/Rigometry/releases).
