<p align="center">
  <img src="assets/rigometry.svg" width="72" height="72" alt="Rigometry monitor and pulse icon">
</p>

# Rigometry

Hardware monitoring and storage analysis for **Windows 11 on Intel and AMD x64 PCs**.

Rigometry is a personal hobby project (Privatprojekt) by Marlon Kaulich, available free of charge with no paid support.

[Download](https://github.com/marlonka/Rigometry/releases) · [User guide](docs/usage.md) · [Build from source](docs/development.md) · [Contribute](CONTRIBUTING.md) · [Impressum](https://marlonkaulich.de/impressum.html)

![Hardware overview in the dark theme](docs/screenshots/overview.png)

## Speed

Ryzen 7 9800X3D, Lexar NM790 4 TB NVMe, `C:\` with 3.2 million files and folders (1.66 TiB on disk).

| | |
| --- | --- |
| Scan the whole drive, as administrator | **2.8–3.6 s** |
| Scan the same drive, standard account | **9–12 s** |
| Search all 3.2 million entries while typing | p50 22–121 ms |
| Memory for 3 million entries | about 1 GB |

As administrator, Rigometry reads the NTFS master file table directly. Without administrator rights, it reads whole folder listings on several threads; that is faster than `robocopy` and used to take 8 minutes. Neither opens a file. [Method, accuracy, comparison with robocopy and how to measure it yourself](docs/benchmarks.md)

## What you can do

- **Watch live activity:** CPU and memory utilization, per-thread CPU activity, GPU utilization and available temperatures, with a 120-second history.
- **Inspect hardware:** processor and cache details, RAM modules, motherboard, BIOS and graphics adapters.
- **Find large files and folders:** scan a whole drive in seconds, with or without administrator rights, browse a sortable hierarchy and size map, search millions of entries with typo-tolerant ranking and `ext:`/`size:` filters, show any entry in Explorer and export CSV or JSON.
- **Check storage accounting:** size on disk and file size, hard links, named streams, exclusions and partial results.
- **Use the interface or command line:** dark/light themes, adjustable text size, reduced motion, hardware reports and automated scans.
- **Choose your language:** English, Deutsch, Français or Español. The app follows the Windows display language by default; change it in Settings without restarting.

Hardware queries and scans are read-only. No account, telemetry upload or bundled driver; administrator rights are requested only when you choose **Scan as administrator**. Settings are saved locally; reports and exports are created when requested.

<details>
<summary>More screenshots</summary>

![GPU sensors and adapter details](docs/screenshots/gpu.png)
![Storage hierarchy](docs/screenshots/storage.png)
![Light theme](docs/screenshots/light-gpu.png)

</details>

## Compatibility

| Requirement | Support |
| --- | --- |
| Operating system | Windows 11, 64-bit |
| Processor | Intel or AMD x86-64; for example Core, Core Ultra, Ryzen or Threadripper on a compatible Windows 11 system |
| ARM processors | No native ARM64 build. Snapdragon and other Windows-on-ARM systems are not supported; x64 emulation is unverified |
| 32-bit processors / Windows | Not supported |
| Other systems | Windows 10, Windows Server, Linux and macOS are not supported application targets |
| Graphics | Working OpenGL-capable vendor graphics driver for the desktop interface; headless reports do not require a graphics session |

The release uses the baseline `x86_64-pc-windows-msvc` target, with no application-wide AVX, AVX2 or AVX-512 requirement and no build-machine-specific CPU optimization. This is an architecture requirement, **not a claim that every CPU model has been tested**. Physical verification currently covers an AMD Ryzen 7 9800X3D, NVIDIA GeForce RTX 5070 Ti and integrated AMD graphics. Intel systems are intended to work through the same Windows/CPUID interfaces but have not been physically verified.

**Sensor support varies.** CPU temperature, voltage and package power, active RAM timings and SPD/XMP/EXPO reading are not implemented. NVIDIA GPU sensors can use the NVML library supplied by an installed NVIDIA driver. AMD and Intel GPUs use Windows interfaces; temperature, fan and clock readings may be unavailable. The app does not change clocks, fan speeds or firmware.

## Install and run

Open [Releases](https://github.com/marlonka/Rigometry/releases), download the Windows x64 ZIP, extract it and run `Rigometry.exe`. Keep the included license and notice files.

Releases include a SHA-256 checksum beside the ZIP and per-file checksums inside it. The executable is unsigned; a checksum verifies file integrity, not publisher identity. No installer is required.

```powershell
# Build from an x64 Developer PowerShell with Rust and C++ Build Tools installed.
cargo build --release --locked
.\target\release\rigometry.exe
```

## Understanding the readings

- **GiB and MiB are binary units:** 1 GiB = 1,073,741,824 bytes. Exports retain byte counts. [Units and accounting](docs/usage.md#units-and-accounting)
- **Unavailable is different from zero.** Missing sensors, driver failures and denied permissions remain explicit.
- **A scan is not total drive usage.** Filesystem bookkeeping, reparse points and cloud placeholders are excluded. Inaccessible or interrupted results remain partial.
- **Some folders need administrator rights.** Without them, Windows hides protected system folders and keeps a few files locked; **Scan as administrator** includes them.

## Development and support

[Development](docs/development.md) · [Architecture](docs/architecture.md) · [Release process](docs/releasing.md) · [Report a bug](https://github.com/marlonka/Rigometry/issues/new/choose) · [Security reporting](SECURITY.md)

Application source and original artwork: [MIT](LICENSE). Dependencies, fonts and runtime components retain their own licenses; see [third-party notices](docs/dependency-notices.md). Developed with AI assistance.

The MIT license also permits commercial use. Provider information for Rigometry: [Impressum](https://marlonkaulich.de/impressum.html).
