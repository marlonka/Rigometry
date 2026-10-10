# Development

[Project](../README.md) · [Architecture](architecture.md) · [Contributing](../CONTRIBUTING.md) · [Releases](releasing.md)

## Build

Requirements: Windows 11 x64, Rustup, PowerShell 7, and Visual Studio C++ Build Tools with the MSVC x64 tools and a Windows SDK. Use an **x64 Developer PowerShell**. NTFS is needed for native filesystem tests.

`rust-toolchain.toml` selects the compiler, Clippy and rustfmt. `Cargo.lock` pins dependencies; the first build needs internet access.

```powershell
rustc -vV
cargo build --release --locked
.\target\release\rigometry.exe
```

The host should be `x86_64-pc-windows-msvc`. The configuration statically links the Microsoft runtime and does not enable build-machine-specific CPU instructions. A Windows SDK resource compiler embeds the application icon, product name and release version.

## Checks

```powershell
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
pwsh -File .\scripts\verify-windows.ps1 -SkipTests
```

Tests use temporary directories. Native fixtures cover hard links, alternate streams, sparse/compressed files, denied access and existing-output protection. UI tests exercise the production interface with fixed hardware inputs and real scan workers. Process tests execute the CLI and verify outputs and failure behavior.

Translation catalogs in `src/i18n/` contain rows of `[English source, German, French, Spanish]`. Keep `{0}`, `{1}` and other argument placeholders intact; translated templates may reorder them. Use presentation helpers for authored text and preserve file paths, device names and provider identifiers as data. Tests check catalog integrity, static interface coverage, language switching, narrow layouts and unchanged exports. Native Windows dialog controls and external error messages remain under Windows or driver control.

The release verification script builds the app, writes a native hardware report and scans a controlled Unicode fixture. Outputs stay in ignored `artifacts/`. It stops only its own child processes.

CI runs formatting, Clippy, tests, release verification, package verification and a RustSec advisory check. Tagged builds additionally prepare a draft GitHub Release. Native hardware accuracy, Narrator and mixed-DPI behavior require manual verification.

Two compatibility jobs rerun the release verification on the built executable:

- **CPU emulation:** under [Intel SDE](https://www.intel.com/content/www/us/en/download/684897/intel-software-development-emulator.html) as Nehalem, Alder Lake, Arrow Lake and Sapphire Rapids. SDE stops on any instruction the emulated chip lacks, so the Nehalem run fails if AVX code reaches the executable. Detected vendor, family/model and instruction sets must match the chip. SDE emulates the instruction set and CPUID identity only: it does not emulate cache leaf 4, and Windows topology, efficiency classes, WMI and SMBIOS still describe the runner. This is not a physical Intel test.
- **Windows on Arm:** the x64 executable under x64 emulation on a `windows-11-arm` runner. Headless only; the runner has no GPU.

To reproduce an emulated run locally, pass the emulator as a command prefix:

```powershell
pwsh -File .\scripts\verify-windows.ps1 -SkipBuild -SkipTests -TimeoutSeconds 900 -Launcher C:\path\to\sde.exe, '-nhm', '--'
```

## Visual checks

```powershell
pwsh -File .\scripts\verify-windows.ps1 -SkipBuild -SkipTests -Capture
```

Inspect affected views in both themes, compact windows and enlarged scale. Scroll to the final rows, check right-edge clearance, keyboard navigation, reduced motion and live-update stability. Screenshot creation alone is not a visual pass. Follow the reusable [design system](design.md).

## Dependencies

For dependency changes, update `Cargo.lock`, [the inventory](dependency-notices.md) and [original notice texts](third-party-licenses.txt). A compiler change also requires refreshing the standard-library notices.

```powershell
cargo install cargo-audit --version 0.22.2 --locked
cargo audit --deny warnings
```

Do not silence advisories to make CI pass. The optional `scripts/audit-dependencies.ps1` also validates inventory coverage, embedded font/NVML notices and common credential patterns. It takes an independently verified cargo-audit executable, RustSec database directory and database commit; inspect its parameters before use. Its output is local evidence, not committed documentation.

## Assets and performance

Regenerate the application icon with `pwsh -File .\scripts\generate-icon.ps1`; inspect small sizes and keep the sidebar geometry synchronized. See [artwork](../assets/README.md).

To refresh screenshots, run `rigometry.exe --language en --capture DIRECTORY --theme Dark --scale 1 --window-size 1440x940 --scan .\src`. Capture mode collects two minutes of live readings, saves five views and closes. It ignores in-app pointer and keyboard input, suppresses hover tooltips, and uses temporary settings. Use `--scan PATH` with a controlled fixture for storage screenshots; existing output files are never overwritten.

`pwsh -File .\scripts\benchmark.ps1` measures startup, idle resource use and a controlled 10,000-file scan. Whole-drive scan and search measurements, with their commands, are in [benchmarks](benchmarks.md); the search benchmark is an ignored test that reads a scan export. State the build, hardware, dataset and cache conditions when sharing measurements.

Keep local reports, downloaded tools, packages, credentials and signing material out of Git. `Cargo.lock`, source artwork and deliberately selected screenshots belong in source control.
