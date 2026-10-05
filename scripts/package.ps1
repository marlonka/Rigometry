#Requires -Version 7.0
[CmdletBinding()]
param(
    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._-]*$')]
    [string]$PackageName = 'Rigometry'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Packaging requires Windows.' }

$workspacePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$workspacePrefix = $workspacePath.TrimEnd('\') + '\'

function Assert-WorkspacePath([string]$Path) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    if ($fullPath -ne $workspacePath -and -not $fullPath.StartsWith($workspacePrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Path escapes the workspace: $fullPath"
    }
    $checkPath = $fullPath
    while ($checkPath -and $checkPath -ne $workspacePath) {
        if (Test-Path -LiteralPath $checkPath) {
            $item = Get-Item -LiteralPath $checkPath -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Reparse path refused: $checkPath" }
        }
        $checkPath = [IO.Path]::GetDirectoryName($checkPath)
    }
    return $fullPath
}

$binaryPath = Assert-WorkspacePath (Join-Path $workspacePath 'target/release/rigometry.exe')
if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
    throw 'Release executable missing. Run cargo build --release --locked first.'
}
$manifest = Get-Content -LiteralPath (Join-Path $workspacePath 'Cargo.toml') -Raw
$versionMatch = [regex]::Match($manifest, '(?m)^version\s*=\s*"([0-9A-Za-z.+-]+)"\s*$')
if (-not $versionMatch.Success) { throw 'Cannot read the application version from Cargo.toml.' }
$version = $versionMatch.Groups[1].Value
$binaryInfo = [Diagnostics.FileVersionInfo]::GetVersionInfo($binaryPath)
if ($binaryInfo.ProductName -cne 'Rigometry' -or $binaryInfo.ProductVersion -cne $version -or $binaryInfo.OriginalFilename -cne 'Rigometry.exe') {
    throw 'Executable product metadata differs from the release identity. Build the current release before packaging.'
}
# Refuse a version-mismatched executable before changing the package directory.
$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $binaryPath
$start.ArgumentList.Add('--version')
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$process = [Diagnostics.Process]::Start($start)
try {
    $output = $process.StandardOutput.ReadToEndAsync()
    $errors = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(10000)) {
        $process.Kill($true)
        throw 'Executable version check timed out. Build the current release first.'
    }
    if ($process.ExitCode -ne 0 -or $output.GetAwaiter().GetResult().Trim() -ne "Rigometry $version") {
        throw 'Executable version differs from Cargo.toml. Build the current release before packaging.'
    }
    [void]$errors.GetAwaiter().GetResult()
} finally {
    $process.Dispose()
}
$packagePath = Assert-WorkspacePath (Join-Path $workspacePath "dist/$PackageName")
$distPath = Assert-WorkspacePath (Join-Path $workspacePath 'dist')
$required = @(
    'README.md', 'LICENSE', 'SECURITY.md', 'CONTRIBUTING.md', 'CHANGELOG.md',
    'assets/README.md', 'assets/rigometry.svg', 'assets/rigometry.png', 'assets/rigometry.ico',
    'docs/README.md', 'docs/usage.md', 'docs/development.md', 'docs/architecture.md',
    'docs/design.md', 'docs/releasing.md', 'docs/dependency-notices.md', 'docs/third-party-licenses.txt',
    'docs/toolchain-notices/rust-1.99.0/README.md',
    'docs/toolchain-notices/rust-1.99.0/COPYRIGHT-library.html',
    'docs/screenshots/overview.png', 'docs/screenshots/gpu.png',
    'docs/screenshots/storage.png', 'docs/screenshots/light-gpu.png'
)
foreach ($relative in $required) {
    $source = Assert-WorkspacePath (Join-Path $workspacePath $relative)
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Required package file missing: $relative" }
}
if (Test-Path -LiteralPath $packagePath) {
    throw "Package directory already exists; use a new PackageName: $packagePath"
}

[IO.Directory]::CreateDirectory($packagePath) | Out-Null
$copied = [Collections.Generic.List[string]]::new()
function Copy-PackageFile([string]$Source, [string]$Relative) {
    $sourcePath = Assert-WorkspacePath $Source
    $destination = Assert-WorkspacePath (Join-Path $packagePath $Relative)
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination)) | Out-Null
    [IO.File]::Copy($sourcePath, $destination, $false)
    $copied.Add($Relative)
}

Copy-PackageFile $binaryPath 'Rigometry.exe'
foreach ($relative in $required) {
    Copy-PackageFile (Join-Path $workspacePath $relative) $relative
}

$checksums = foreach ($relative in $copied | Sort-Object) {
    $filePath = Assert-WorkspacePath (Join-Path $packagePath $relative)
    $hash = (Get-FileHash -LiteralPath $filePath -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $($relative.Replace('\', '/'))"
}
$checksumPath = Assert-WorkspacePath (Join-Path $packagePath 'SHA256SUMS.txt')
[IO.File]::WriteAllLines($checksumPath, $checksums, [Text.UTF8Encoding]::new($false))
$copied.Add('SHA256SUMS.txt')

$stamp = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ')
$zipPath = Assert-WorkspacePath (Join-Path $distPath "Rigometry-$version-windows-x64-$stamp.zip")
if (Test-Path -LiteralPath $zipPath) { throw "Archive already exists: $zipPath" }
$archive = [IO.Compression.ZipFile]::Open($zipPath, [IO.Compression.ZipArchiveMode]::Create)
try {
    # Only explicitly listed distribution files enter the archive.
    foreach ($relative in $copied | Sort-Object) {
        $source = Assert-WorkspacePath (Join-Path $packagePath $relative)
        $entry = 'Rigometry/' + $relative.Replace('\', '/')
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $source, $entry, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
} finally {
    $archive.Dispose()
}
$zipHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
$sidecarPath = Assert-WorkspacePath "$zipPath.sha256"
[IO.File]::WriteAllText($sidecarPath, "$zipHash  $([IO.Path]::GetFileName($zipPath))`n", [Text.UTF8Encoding]::new($false))
[pscustomobject]@{ Package = $packagePath; Archive = $zipPath; SHA256 = $zipHash; Checksum = $sidecarPath }
