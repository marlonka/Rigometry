# Benchmarks

[Back to the project](../README.md)

Measured on 10 October 2026 with a release build. The PC has an AMD Ryzen 7 9800X3D, 32 GB RAM and Windows 11 Pro. `C:\` is NTFS on a Lexar NM790 4 TB NVMe SSD and holds 3.2 million files and folders, 1.66 TiB on disk. Caches were warm from earlier scans; nothing else was controlled.

## Whole-drive scan

| Method | Entries | Time |
| --- | ---: | ---: |
| Rigometry, **Scan as administrator** (reads the NTFS master file table) | 3.2 million | **2.8–3.6 s** |
| Rigometry, standard account (reads folder listings, 8 threads) | 3.1 million | **9.4–12.3 s** |
| `robocopy /L /S /MT:16`, 16 threads, names and file sizes only | 2.6 million files | 15 s |
| Rigometry 0.1.0, standard account (opened every file) | 3.1 million | 8 min |

The administrator scan reads names, folders and sizes for the whole drive straight from the master file table. Its result includes what a standard account cannot read: protected folders such as `WindowsApps`, restore points and named data streams. The two runs were 2.8 s and 3.6 s.

A standard account reads each folder's listing, which holds the name, file size, size on disk and file ID of every entry. Eight threads list folders in parallel, and no file is opened, so files Windows keeps open, such as `pagefile.sys`, show their size too. Three runs took 12.3, 10.2 and 9.4 s; the first run after other disk activity took 21 s. Most of the remaining time is Windows opening each of the 511,000 folders: it checks permissions and passes every open through file-system filters such as Microsoft Defender. Opening folders relative to their parent was measured to save only 15–28% of that, so Rigometry does not.

All times cover reading, parsing and building the tree; they exclude writing the export. robocopy is the fastest built-in Windows comparison: it lists names and file sizes, not size on disk.

### Do the methods agree?

**Folder listings and the per-file method of 0.1.0**, compared file by file across one user profile: 2,018,868 files.

| | Folder listings | Per-file method |
| --- | ---: | ---: |
| File size | 740.663 GiB | 740.664 GiB |
| Size on disk | 744.045 GiB | 744.037 GiB |

File sizes are identical except for 2,889 files with named data streams, such as download markers: 1 MiB together, which folder listings leave out. Size on disk is identical for 99.7% of files; the rest differ by 8.9 MiB in total. Those are files written to during the comparison, and compressed files, for which the listing reports whole clusters while the per-file method reported compressed bytes. Under `C:\Windows\System32`, including CompactOS files, all 19,473 files matched exactly.

**The administrator scan and the per-file method**, compared file by file under `C:\Users`, in scans a few minutes apart.

| | Administrator scan | Per-file method |
| --- | ---: | ---: |
| Files found in both | 2,141,933 | 2,141,933 |
| File size | 741.81 GiB | 741.81 GiB |
| Size on disk | 740.14 GiB | 740.27 GiB |

File sizes agree except for 70 files that changed between the runs. Size on disk differs by 0.13 GiB, for known reasons:
- Files under 1 KiB are stored inside the master file table and occupy no clusters. The administrator scan reports 0 for them, as Explorer does; the per-file method reports Windows' rounded value. Together 149 MiB.
- For compressed and sparse files, the administrator scan counts whole clusters. Together 10 MiB.
- About 1,500 files existed in only one of the runs, mostly in folders that were being written to at the time.

Whole folders match exactly where both methods can read everything: `Games` 31.7 GiB and `Program Files (x86)` 419.3 GiB, each with the same file count.

## Search

Each query, as typed into the Storage search, ran 30 times over the scan above, all 3.2 million entries. The times include parsing, matching and ranking.

| Query | Results | p50 | p95 |
| --- | ---: | ---: | ---: |
| `ext:mkv size:>1g` | 67 | 22 ms | 22 ms |
| `kind:dir node_modules` | 2,772 | 28 ms | 35 ms |
| `docker` | 8,881 | 49 ms | 58 ms |
| `gemma gguf` | 473 | 83 ms | 90 ms |
| `reprot` (typo for "report") | 28,617 | 100 ms | 105 ms |
| `readme !node_modules` | 46,644 | 102 ms | 108 ms |
| `path:\steamapps\ ext:pak` | 554 | 121 ms | 132 ms |

Search runs on the scan in memory, so it needs no index and finds entries while a scan is still running. The first query after a scan also computes each name's character mask, once: 67 ms here.

## Memory

A scan keeps every entry in memory: about 1 GB for 3 million entries (peak private memory, standard scan). Scans stop at a 1.5 GiB budget, about 5 million entries, and say so.

## Measure it yourself

Whole-drive scan; `summary.elapsed_ms` in the export is the scan time. From an administrator PowerShell it reads the master file table, otherwise folder listings:

```powershell
.\target\release\rigometry.exe --headless --scan C:\ --export .\scan.json
```

Search, on that export:

```powershell
$env:RIGOMETRY_BENCH_SCAN = (Resolve-Path .\scan.json)
cargo test --release bench_search -- --ignored --nocapture
```

robocopy, which lists without copying:

```powershell
Measure-Command { robocopy C:\ C:\__nowhere /L /S /XJ /R:0 /W:0 /NFL /NDL /NJH /BYTES /MT:16 }
```

`scripts\benchmark.ps1` measures startup, idle CPU and memory, and a controlled 10,000-file scan.

Exports contain every path on the drive. Delete them after measuring, and do not share them.
