# User guide

[Back to Rigometry](../README.md)

## Install on macOS

Rigometry for Mac is one universal app for Apple silicon (M1 and later) and Intel Macs, macOS 11 Big Sur or later.

1. Open [Releases](https://github.com/marlonka/Rigometry/releases/latest) and download `Rigometry-<version>-macos-universal.zip`.
2. Double-click the ZIP in **Downloads**. Drag **Rigometry.app** from the extracted folder into **Applications**.
3. Double-click Rigometry in Applications. Because the app is not notarized by Apple, macOS first shows a warning that it cannot verify the app. Click **Done** (or **OK**).
4. Open **System Settings → Privacy & Security**, scroll to **Security** and click **Open Anyway** next to the message about Rigometry. Confirm with your password or Touch ID, then click **Open**.

From then on, Rigometry opens normally. On macOS 14 Sonoma or earlier, you can instead Control-click the app, choose **Open** and confirm.

To check the download first, open Terminal in the Downloads folder and run `shasum -a 256 -c Rigometry-<version>-macos-universal.zip.sha256` after downloading the `.sha256` file beside the ZIP. It should print `OK`.

On a Mac, Rigometry shows CPU and memory activity, processor details, drives and the full storage analysis. **Show in Finder** replaces Explorer. GPU sensors, motherboard and RAM-module details, and **Scan as administrator** are Windows-only. Scans read whole folder listings on up to eight threads with `getattrlistbulk`, the macOS counterpart of the Windows folder listing, and open no file. Scanning `/` covers the startup disk once, including `/Users` and `/Applications` on the data volume, and does not enter other mounted volumes such as `/System/Volumes/Data`, `/Volumes` or `/dev`; scan those directly. Files stored only in iCloud count as cloud placeholders and are not downloaded. The first scan of Desktop, Documents or Downloads makes macOS ask whether Rigometry may access that folder. Folders macOS protects, such as Mail or Safari data, need **Full Disk Access** for Rigometry under **System Settings → Privacy & Security**; without it they count as refused, not as errors. Shortcuts use ⌘ instead of Ctrl, for example ⌘1 to ⌘4.

## Updates

Rigometry asks GitHub for the latest release five seconds after it starts and again every six hours while it is open. When a newer version is out, **Update x.y.z available** appears at the bottom of the sidebar, and **Settings → Updates** shows what is new.

- **Update and restart** downloads the release's ZIP for your system from this repository's releases, checks its size and the SHA-256 checksum GitHub publishes for it, unpacks it with the system's own tools and makes sure the new version starts. Only then does it replace the app: on Windows the files in the app's folder, on a Mac `Rigometry.app`. The new version starts once the old one has closed and saved its settings.
- Nothing is installed without your click, and a scan, duplicate search or export must finish first. **Cancel**, or closing the window, stops the update at any point before files are replaced; an update that is already replacing files completes, and the new version is used the next time. If the new version cannot be started, **Restart now** tries again.
- **Skip this version** hides the sidebar notice until the next version. **Release page** opens the release on GitHub.
- The app updates itself only where it can write: the folder you unpacked on Windows, `Rigometry.app` in **Applications** on a Mac. A Mac app started straight from **Downloads** runs from a read-only copy; move it to **Applications** first. If macOS refuses the change, allow Rigometry under **System Settings → Privacy & Security → App Management**, or download the update from the release page. On Windows, a copy started with administrator rights does not update itself; restart it normally. A copy built from the source code shows the release page instead.
- Turn off **Check for new versions automatically** to stop the automatic checks; **Check now** still asks. A failed automatic check, for example offline, is tried again after 30 minutes and shows no error. The checks send nothing beyond the request itself, which GitHub sees with your IP address and the app's version. Setting `RIGOMETRY_NO_UPDATE_CHECK` turns them off as well.

Downloads use the system's `curl`, which Windows 10 and later and macOS include, with its certificate store. Rigometry itself contains no network code beyond starting it.

## Navigation

| Action | Shortcut |
| --- | --- |
| Overview, CPU & Memory, GPU, Storage | `Ctrl+1` through `Ctrl+4` |
| Rescan the current storage path | `F5` |
| Cancel a storage scan or duplicate search | `Esc` (after leaving a text field or menu) |
| Parent storage folder | `Alt+Up` |

Settings includes dark/light/system themes, scale from 85–150% in 5% steps and reduced motion, which also turns off animated scrolling. At smaller window sizes, sections stack and the page scrolls vertically. When a new version is out, the sidebar notice opens Settings at **Updates**.

## Language

Choose **Settings → Language**: English, Deutsch, Français, Español or System language. Switching applies immediately and is saved for the next launch. System language follows the Windows display language or the first preferred language in macOS, including regional variants; other languages fall back to English.

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

Installed RAM is firmware capacity. Memory available to the operating system may be smaller; memory in use is a live operating-system reading. These are separate measurements.

Storage sizes are **On disk**: the clusters an entry occupies, which is what deleting it frees. Each file identity counts once, attributed to the first scanned hard-link path. **File size** (the sum of file lengths, including named streams and every hard-link path) appears for the selected entry, in tooltips and in exports; the page shows it for the current folder only where it differs noticeably, for example for NTFS-compressed Windows folders. Directory indexes, filesystem metadata and other volume bookkeeping are excluded. Drive usage and scan allocation answer different questions.

Scans read folder listings and open no file, so files Windows keeps open exclusively, such as `pagefile.sys`, show their size too. Named data streams, such as the download markers Windows attaches to files from the internet, count only in administrator scans of a whole drive.

Right-click any row or map tile to show it in Explorer (Finder on a Mac), copy its path or open a folder; the folder button at the end of each row also shows the entry there.

The size map shows the entries of the current folder, each tile's area proportional to its size on disk. Folders are gray; files are colored by what they hold, judged by their extension the same way as the `type:` search filter: media, documents, code, archives and disk images, programs, and other files. The legend lists only the kinds present.

`≥` marks a known lower bound. Cancellation, inaccessible entries or a metadata limit can leave totals incomplete. Expand scan accounting to inspect exclusions and errors.

Without administrator rights, Windows refuses access to some system folders, such as `System Volume Information` and parts of `C:\Windows`. The status line counts them as entries that need administrator rights, not as errors. **Scan as administrator** restarts Rigometry with administrator rights after Windows asks for consent, then scans the same path again, still reading metadata only. On a whole NTFS drive, an administrator scan reads the drive's master file table directly, about three times faster than folder listings. Folder scans and other file systems read folder listings.

## Storage search

The **Search** field on the Storage page searches every entry below the current folder, including entries that are still arriving during a scan. Results are ranked by relevance until you choose a column; clearing the field returns to the hierarchy and the previous sort.

- **Words match names, in any order.** `inv 2025` finds `Invoice_2025.pdf`. Matches at word starts, whole names and names without the extension rank first.
- **Typos are forgiven.** Words of five or more letters tolerate one wrong, missing, extra or swapped letter (`reprot` finds `report.docx`). Digits are never treated as typos.
- **Folders count.** `projects report` finds `report.bin` inside a `Projects` folder. At least one word must match the entry's own name. The current folder's own name is not part of the search.
- **Modifiers:** `'exact`, `^prefix`, `suffix$`, `!exclude` (also hides everything inside an excluded folder) and `"two words"` for a phrase.

| Filter | Example | Selects |
| --- | --- | --- |
| `ext:` | `ext:mp4,mkv` | Files with these extensions |
| `type:` | `type:video` | `image`, `video`, `audio`, `doc`, `code`, `archive`, `disk`, `app` or `font` extensions |
| `kind:` | `kind:dir` | `file` or `dir` |
| `size:` | `size:>500m`, `size:1g..4g` | Size on disk; `>`, `>=`, `<`, `<=`, ranges or an exact value |
| `path:` | `path:\steam\` | Full path contains the text; a pasted `C:\…` path works the same way |

Size suffixes follow [the units above](#units-and-accounting): `k`, `m`, `g`, `t` and `kib`…`tib` are binary, as shown in the table; `kb`, `mb`, `gb`, `tb` are decimal. `size:` on a folder uses its total size on disk, so `kind:dir size:>10g` finds large folders. An invalid filter value is reported below the field.

Search runs on the scan already in memory: it never reads the disk, so it only finds what the current scan includes. Repeated filters narrow each other: `size:>1g size:<4g` finds sizes between the two. On a Mac, a pasted absolute path such as `/Users/me/Projects` works like `path:`. Scoring and query syntax are adapted from [FSearch](https://github.com/noahdunnagan/fsearch) (MIT).

## Duplicate files

After a scan, open **Duplicates** on the Storage page, choose the **Smallest file** to compare and click **Find duplicates**. Rigometry compares the files below the current folder; nothing is changed or deleted, and the scan itself still opens no file.

- **Copies have identical contents.** Only files of equal size are read. Rigometry first hashes the first and last 16 KiB of each, then reads in full only the files that still match another, and compares them by [BLAKE3](https://github.com/BLAKE3-team/BLAKE3) hash. Equal hashes mean equal contents; names, dates and folders play no part.
- **The biggest savings come first.** Groups appear as they are confirmed, sorted by the space keeping one copy would free. The oldest copy, likely the original, is listed first; hover a copy for its full path.
- **Hard links are not copies.** Further names of one file are counted once and free nothing. On a Mac, clones that share storage on APFS count only their unshared part.
- **Use the search to narrow the list.** `ext:jpg` shows only groups with a JPEG copy; the other filters work as described above. The totals and the export follow the narrowed list; a JSON export records the search.
- **Contents means the file's data.** Windows download markers and other named streams, and resource forks on a Mac, are not compared.
- **Changed files are left out.** A file that changed, moved or became a link since the scan, or while it was read, is not reported and is counted under **Comparison details**. So are files that could not be read.
- **Cloud files stay in the cloud.** Files stored only in OneDrive or iCloud are skipped and never downloaded. On a Mac, comparing does not change files' access times.
- **Export the result** as CSV or JSON: each copy with its path, size, BLAKE3 hash and modification time.

Rigometry reads one file at a time on hard disks and several at once on SSDs. Reading is limited by the drive: expect roughly the drive's sequential speed for the files that need a full read. Windows may update the last-access time of files it reads where that feature is enabled. On ReFS volumes, such as Dev Drives, copies can already share blocks, so deleting one may free less than shown.

## Sensors

The app records a 120-second window. Minimum and maximum values use that recorded history. Missing samples remain gaps; a reading becomes stale after five seconds without an update. Firmware specifications and live measurements are labeled separately.

Choose the GPU adapter before comparing readings. Device-wide usage/capacity and process-local memory usage/budgets are different fields. See [compatibility and sensor limits](../README.md#compatibility).

## Command line

Use `--help` for options and `--version` for the build version. Output parent directories must exist; output files must have new names, and every output is checked before any work starts. Existing files and aliases are not intentionally replaced. `.csv` selects CSV regardless of case; other export extensions select JSON.

The exit code is 0 on success and 1 on any error, including a scan whose folder cannot be read. Partial results, such as folders that needed administrator rights, still succeed; their `status` field says `partial`.

The Windows release is a desktop application, so PowerShell does not wait for it. To wait and read the exit code, start it with `Start-Process -Wait -NoNewWindow -PassThru` and check `.ExitCode`. On a Mac, run `Rigometry.app/Contents/MacOS/rigometry` from Terminal.

```powershell
New-Item -ItemType Directory -Force .\artifacts | Out-Null

# Inventory and one measured sample after a baseline interval.
.\target\release\rigometry.exe --headless --report .\artifacts\hardware.json

# Folder scan with an explicit output file.
.\target\release\rigometry.exe --headless --scan 'C:\Data' --export .\artifacts\scan.json
.\target\release\rigometry.exe --headless --scan 'C:\Data' --export .\artifacts\scan.csv

# Duplicate files of at least 4 MiB below a folder (default 1 MiB).
.\target\release\rigometry.exe --headless --scan 'C:\Data' --duplicates .\artifacts\copies.csv --min-size 4m

# Open the desktop app and begin scanning.
.\target\release\rigometry.exe --scan 'C:\Data'
```

Exports contain paths and hardware details. Review before sharing. Concurrent filesystem changes are possible; scans are observations, not atomic snapshots. Cancellation cannot interrupt every blocked Windows API call.

## Screenshots

Capture mode renders five application views, then closes. It requires a graphics session and new output names:

```powershell
.\target\release\rigometry.exe --capture .\artifacts\screenshots --scan .\src
.\target\release\rigometry.exe --capture .\artifacts\screenshots-light --theme Light --scale 1.5
.\target\release\rigometry.exe --capture .\artifacts\screenshots-small --window-size 1080x720 --scale 1.5
```

Capture output is evidence for manual inspection; successful file creation does not establish visual quality or accessibility.
