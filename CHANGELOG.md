# Changelog

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
