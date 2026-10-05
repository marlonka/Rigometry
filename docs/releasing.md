# Releases

[Project](../README.md) · [Development](development.md)

## Branches and versions

`main` is the default branch and contains the maintained source. Use short-lived branches for changes and merge reviewed work into `main`.

Use semantic versions in `Cargo.toml` and `Cargo.lock`. A release tag is `v` followed by that exact version, such as `v0.1.0`. Published tags and assets are immutable: correct a faulty release with a new version, not a replaced ZIP.

Keep current release notes in [CHANGELOG.md](../CHANGELOG.md). Describe observable changes and known limits, without copying development logs.

## Prepare

1. Update the version and release notes. Keep dependency and runtime notices current.
2. Run the [development checks](development.md#checks) and inspect affected interfaces with real hardware.
3. Confirm the README's CPU, Windows and sensor requirements match the build. Do not claim untested hardware as verified.
4. Commit and push to `main`. Tag that exact commit and push only the new tag.

```powershell
git tag -a v0.1.0 -m "Rigometry 0.1.0"
git push origin v0.1.0
```

Replace the example with the next actual version. Never reuse an existing tag.

## Automated package

The CI workflow runs on branch and tag pushes. A version tag must match the manifest before the build proceeds. Formatting, Clippy, tests, native verification, package checks and dependency advisories must all pass.

For tags, the workflow uploads the verified ZIP and its checksum to a **draft GitHub Release**. The package includes the executable, user guide, license inventory, original third-party notices and per-file checksums. Hardware reports and local audit logs are excluded.

The build job has read-only repository access. A separate job receives release-write permission only after the checks pass. Third-party workflow actions are pinned to commit IDs.

## Publish

Open the draft in [Releases](https://github.com/marlonka/Rigometry/releases). Confirm its commit, asset names, checksum, compatibility notes and distribution permissions. Enable private vulnerability reporting before making the repository public.

Publish the draft when it is ready for users; mark preview builds as pre-releases. Creating or publishing a release does not change repository visibility. The workflow does not automatically make a draft public.

## Local package

```powershell
cargo build --release --locked
$package = & .\scripts\package.ps1 -PackageName Rigometry-local
& .\scripts\verify-package.ps1 -PackagePath $package.Package -ArchivePath $package.Archive
```

Packaging uses an explicit file list, refuses an existing output directory and checks the executable version. The verifier checks every manifest entry and ZIP byte stream. Use a new package name for another build.

The executable is unsigned. Checksums establish file integrity, not publisher identity. Do not disable Windows security controls as an installation step.