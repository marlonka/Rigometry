#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$PackagePath,
    [Parameter(Mandatory)][string]$ArchivePath
)

# Read-only verification. No executable from the package or archive is run.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$package = (Resolve-Path -LiteralPath $PackagePath).ProviderPath
$archive = (Resolve-Path -LiteralPath $ArchivePath).ProviderPath
$prefix = $package.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
$manifestPath = Join-Path $package 'SHA256SUMS.txt'
$expected = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::OrdinalIgnoreCase)

foreach ($line in Get-Content -LiteralPath $manifestPath) {
    if ($line -notmatch '^([a-fA-F0-9]{64})  (.+)$') { throw 'Invalid checksum manifest line.' }
    $hash, $relative = $Matches[1], $Matches[2]
    if ($relative -match '(^/|\\|:|(^|/)\.\.?(/|$))') { throw "Unsafe manifest path: $relative" }
    $path = [IO.Path]::GetFullPath((Join-Path $package $relative))
    if (-not $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Manifest path escapes package.' }
    if (-not $expected.TryAdd("Rigometry/$relative", $hash)) { throw "Duplicate manifest path: $relative" }
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $hash) { throw "Package checksum mismatch: $relative" }
}
if ($expected.Count -eq 0 -or -not $expected.ContainsKey('Rigometry/Rigometry.exe')) { throw 'Package executable is missing from the manifest.' }
$expected.Add('Rigometry/SHA256SUMS.txt', (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash)

$sidecar = (Get-Content -LiteralPath "$archive.sha256" -Raw).Trim()
if ($sidecar -notmatch '^([a-fA-F0-9]{64})  (.+)$') { throw 'Invalid archive checksum sidecar.' }
if ($Matches[2] -cne [IO.Path]::GetFileName($archive)) { throw 'Archive checksum names another file.' }
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $Matches[1]) { throw 'Archive checksum mismatch.' }

$zip = [IO.Compression.ZipFile]::OpenRead($archive)
try {
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in $zip.Entries) {
        if (-not $seen.Add($entry.FullName) -or -not $expected.ContainsKey($entry.FullName)) { throw "Unexpected or duplicate archive entry: $($entry.FullName)" }
        $stream = $entry.Open()
        try { $hash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)) }
        finally { $stream.Dispose() }
        if ($hash -ne $expected[$entry.FullName]) { throw "Archived checksum mismatch: $($entry.FullName)" }
    }
    if ($seen.Count -ne $expected.Count) { throw 'Archive is missing expected package files.' }
} finally { $zip.Dispose() }
Write-Output "Verified $($expected.Count - 1) packaged files, archive entries and ZIP checksum."
