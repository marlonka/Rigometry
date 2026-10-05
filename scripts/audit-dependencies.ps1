#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$CargoAuditPath,
    [Parameter(Mandatory)][string]$DatabasePath,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{40}$')][string]$DatabaseRevision,
    [ValidatePattern('^[0-9a-fA-F]{64}$')][string]$ExpectedToolSha256 = '0157f5ce1ce9fd4fb0a1f7c79af1229771d1f80b6c2613ddb0d9200a8ba73946',
    [switch]$NoYanked
)

# Read-only dependency audit. Does not install tools, change Cargo.lock, update
# dependencies, modify Git, or publish. The caller supplies an independently
# downloaded RustSec database; DatabaseRevision records that provenance and is
# not a cryptographic verification of the extracted database directory.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$auditExe = (Resolve-Path -LiteralPath $CargoAuditPath).ProviderPath
$database = (Resolve-Path -LiteralPath $DatabasePath).ProviderPath
if (-not (Test-Path -LiteralPath $auditExe -PathType Leaf)) { throw 'CargoAuditPath must identify the audit executable.' }
if (-not (Test-Path -LiteralPath (Join-Path $database 'crates') -PathType Container)) { throw 'DatabasePath must contain the RustSec crates directory.' }
$toolHash = (Get-FileHash -LiteralPath $auditExe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($toolHash -ne $ExpectedToolSha256.ToLowerInvariant()) { throw 'Audit executable SHA-256 does not match the expected value.' }

function Get-CargoOutput([string[]]$Arguments) {
    $result = & cargo @Arguments
    if ($LASTEXITCODE -ne 0) { throw "cargo $($Arguments -join ' ') failed ($LASTEXITCODE)." }
    return $result
}

Push-Location -LiteralPath $workspace
try {
    $artifactRoot = Join-Path $workspace 'artifacts'
    $auditRoot = Join-Path $artifactRoot 'dependency-audit'
    foreach ($path in @($artifactRoot, $auditRoot)) {
        if ((Test-Path -LiteralPath $path) -and ((Get-Item -LiteralPath $path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Reparse output path refused: $path"
        }
    }
    $stamp = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $run = Join-Path $auditRoot $stamp
    [IO.Directory]::CreateDirectory($run) | Out-Null

    $metadataText = (Get-CargoOutput @('metadata', '--locked', '--offline', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc')) -join "`n"
    [IO.File]::WriteAllText((Join-Path $run 'metadata-windows.json'), $metadataText, [Text.UTF8Encoding]::new($false))
    $metadata = $metadataText | ConvertFrom-Json
    $resolved = @{}
    foreach ($node in $metadata.resolve.nodes) { $resolved[$node.id] = $true }
    $packages = @($metadata.packages | Where-Object { $resolved.ContainsKey($_.id) -and $_.id -ne $metadata.resolve.root })

    $tree = @(Get-CargoOutput @('tree', '--locked', '--offline', '--target', 'x86_64-pc-windows-msvc', '--edges', 'normal,build', '--prefix', 'none', '--no-dedupe'))
    [IO.File]::WriteAllLines((Join-Path $run 'tree-release-and-build.txt'), $tree, [Text.UTF8Encoding]::new($false))
    $rootPackage = $metadata.packages | Where-Object id -eq $metadata.resolve.root
    $release = @($tree | ForEach-Object { if ($_ -match '^([A-Za-z0-9_-]+) v([^\s]+)') { "$($Matches[1]) $($Matches[2])" } } | Sort-Object -Unique | Where-Object { $_ -ne "$($rootPackage.name) $($rootPackage.version)" })
    $developmentOnly = @($packages | Where-Object { "$($_.name) $($_.version)" -notin $release } | ForEach-Object { "$($_.name) $($_.version)" })

    $inventory = Get-Content -LiteralPath 'docs/dependency-notices.md' -Raw
    $notices = (Get-Content -LiteralPath 'docs/third-party-licenses.txt' -Raw).Replace("`r`n", "`n")
    $missingInventory = @($packages | Where-Object { -not $inventory.Contains("https://crates.io/crates/$($_.name)/$($_.version)") } | ForEach-Object { "$($_.name) $($_.version)" })
    $missingAttribution = @($packages | Where-Object { -not $notices.Contains("$($_.name) $($_.version)") } | ForEach-Object { "$($_.name) $($_.version)" })
    $missingLicense = @($packages | Where-Object { -not $_.license } | ForEach-Object { "$($_.name) $($_.version)" })
    $fontChecks = @()
    $fonts = $packages | Where-Object name -eq 'epaint_default_fonts'
    if ($fonts) {
        $fontFolder = Join-Path (Split-Path $fonts.manifest_path) 'fonts'
        $fontChecks = @(Get-ChildItem -LiteralPath $fontFolder -File -Filter '*.txt' | ForEach-Object {
            $body = (Get-Content -LiteralPath $_.FullName -Raw).Trim().Replace("`r`n", "`n")
            [pscustomobject]@{ File = $_.Name; ExactNoticeIncluded = $notices.Contains($body) }
        })
    }
    $nvmlComments = @()
    foreach ($line in Get-Content -LiteralPath 'src/hardware/nvml.rs') {
        if ($line.StartsWith('//')) { $nvmlComments += $line.TrimStart('/').TrimStart('!').Trim() }
        elseif ($line.Trim()) { break }
    }

    $auditArguments = @('audit', '--no-fetch', '--db', $database, '--format', 'json', '--file', 'Cargo.lock')
    if ($NoYanked) { $auditArguments += '--no-yanked' }
    & $auditExe @auditArguments 1> (Join-Path $run 'rustsec.json') 2> (Join-Path $run 'rustsec-stderr.txt')
    $auditExit = $LASTEXITCODE
    $auditReport = Get-Content -LiteralPath (Join-Path $run 'rustsec.json') -Raw | ConvertFrom-Json
    if (-not $auditReport) { throw "Audit did not return JSON; inspect $run/rustsec-stderr.txt" }

    $tracked = @(& git ls-files)
    if ($LASTEXITCODE -ne 0) { throw 'git ls-files failed.' }
    $binaryOrCredentialPaths = @($tracked | Where-Object { $_ -match '(?i)(\.exe$|\.dll$|\.sys$|\.zip$|\.ttf$|\.ttc$|\.pfx$|\.key$|\.env$)' })
    # Return filenames only; never copy matched secret material into the report.
    $secretCandidates = @(& git grep -Il -E 'BEGIN (RSA |EC |OPENSSH |DSA )?PRIVATE KEY|github_pat_[A-Za-z0-9_]+|gh[pousr]_[A-Za-z0-9]{25,}|sk-[A-Za-z0-9]{30,}|AKIA[A-Z0-9]{16}')
    if ($LASTEXITCODE -gt 1) { throw 'Tracked-file secret-pattern scan failed.' }
    $gitCommit = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'git rev-parse failed.' }
    $summary = [ordered]@{
        measured_at_utc = [DateTime]::UtcNow.ToString('o')
        git_commit = $gitCommit
        lockfile_sha256 = (Get-FileHash -LiteralPath Cargo.lock -Algorithm SHA256).Hash.ToLowerInvariant()
        audit_tool_sha256 = $toolHash
        database_revision_recorded_by_caller = $DatabaseRevision.ToLowerInvariant()
        yanked_check_enabled = -not $NoYanked
        rustsec_exit_code = $auditExit
        rustsec = $auditReport
        windows_all_resolved_dependency_versions = $packages.Count
        windows_release_and_build_dependency_versions = $release.Count
        windows_development_only_versions = $developmentOnly
        missing_inventory_entries = $missingInventory
        missing_notice_attributions = $missingAttribution
        missing_declared_licenses = $missingLicense
        embedded_font_notice_checks = $fontChecks
        nvml_source_notice_in_documentation = $notices.Contains(($nvmlComments -join "`n"))
        tracked_binary_or_credential_paths = $binaryOrCredentialPaths
        tracked_secret_pattern_candidates = $secretCandidates
        limitations = @('Package attribution checks do not interpret license obligations or prove complete license-text coverage.', 'Graph counts include build/proc-macro code and do not establish which symbols reach the executable.', 'RustSec result covers the lockfile, not bundled Rust standard-library or Microsoft runtime code.', 'Secret patterns are heuristics over currently tracked text, not a complete secret or history audit.')
    }
    $summaryPath = Join-Path $run 'summary.json'
    $summary | ConvertTo-Json -Depth 50 | Set-Content -LiteralPath $summaryPath -Encoding utf8
    [pscustomobject]@{ Summary = $summaryPath; AuditExitCode = $auditExit; WindowsReleaseAndBuild = $release.Count; WindowsAll = $packages.Count }
    if ($auditExit -ne 0 -or $missingInventory.Count -or $missingAttribution.Count -or $missingLicense.Count -or @($fontChecks | Where-Object { -not $_.ExactNoticeIncluded }).Count -or -not $summary.nvml_source_notice_in_documentation) {
        throw "Dependency audit requires attention; inspect $summaryPath"
    }
} finally {
    Pop-Location
}
