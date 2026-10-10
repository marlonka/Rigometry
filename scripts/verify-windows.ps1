#Requires -Version 7.0
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$SkipTests,
    [switch]$Capture,
    [string]$ScanPath,
    # Command prefix that runs the executable, e.g. an emulator: sde.exe, -nhm, --
    [string[]]$Launcher = @(),
    [ValidateRange(30, 3600)][int]$TimeoutSeconds = 300
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Native verification requires Windows.' }
$workspacePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$workspacePrefix = $workspacePath.TrimEnd('\') + '\'

function Assert-WorkspacePath([string]$Path) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    if ($fullPath -ne $workspacePath -and -not $fullPath.StartsWith($workspacePrefix, [StringComparison]::OrdinalIgnoreCase)) { throw "Output escapes the workspace: $fullPath" }
    $checkPath = $fullPath
    while ($checkPath -and $checkPath -ne $workspacePath) {
        if (Test-Path -LiteralPath $checkPath) {
            if ((Get-Item -LiteralPath $checkPath -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Reparse output path refused: $checkPath" }
        }
        $checkPath = [IO.Path]::GetDirectoryName($checkPath)
    }
    return $fullPath
}

function Invoke-Cargo([string[]]$CargoArguments) {
    & cargo @CargoArguments
    if ($LASTEXITCODE -ne 0) { throw "cargo $($CargoArguments -join ' ') failed ($LASTEXITCODE)." }
}

function Invoke-Application([string[]]$ApplicationArguments) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = if ($Launcher.Count -gt 0) { $Launcher[0] } else { $binaryPath }
    $start.WorkingDirectory = $workspacePath
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    if ($Launcher.Count -gt 0) {
        foreach ($value in @($Launcher | Select-Object -Skip 1)) { $start.ArgumentList.Add($value) }
        $start.ArgumentList.Add($binaryPath)
    }
    foreach ($value in $ApplicationArguments) { $start.ArgumentList.Add($value) }
    $process = [Diagnostics.Process]::Start($start)
    try {
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $process.Kill($true)
            throw "Rigometry exceeded $TimeoutSeconds seconds. Its verification process was stopped."
        }
        if ($process.ExitCode -ne 0) { throw "Rigometry exited with code $($process.ExitCode)." }
    } finally {
        $process.Dispose()
    }
}

Push-Location -LiteralPath $workspacePath
try {
    if (-not $SkipBuild -or -not $SkipTests) {
        $compiler = & rustc -vV
        if ($LASTEXITCODE -ne 0 -or -not ($compiler -match '^host: x86_64-pc-windows-msvc$')) { throw 'Use the x86_64-pc-windows-msvc Rust toolchain from an x64 Developer PowerShell.' }
        $compiler | Write-Output
    }
    if (-not $SkipTests) { Invoke-Cargo @('test', '--locked') }
    if (-not $SkipBuild) { Invoke-Cargo @('build', '--release', '--locked') }
    $binaryPath = Assert-WorkspacePath (Join-Path $workspacePath 'target/release/rigometry.exe')
    if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) { throw 'Release executable missing.' }

    $runName = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $runPath = Assert-WorkspacePath (Join-Path $workspacePath "artifacts/verification/$runName")
    [IO.Directory]::CreateDirectory($runPath) | Out-Null
    $fixture = [string]::IsNullOrWhiteSpace($ScanPath)
    if ($fixture) {
        $ScanPath = Assert-WorkspacePath (Join-Path $runPath 'fixture')
        $unicodePath = Assert-WorkspacePath (Join-Path $ScanPath '測定')
        [IO.Directory]::CreateDirectory($unicodePath) | Out-Null
        [IO.Directory]::CreateDirectory((Assert-WorkspacePath (Join-Path $ScanPath 'empty'))) | Out-Null
        [IO.File]::WriteAllBytes((Assert-WorkspacePath (Join-Path $ScanPath 'alpha.bin')), [byte[]]::new(4096))
        [IO.File]::WriteAllBytes((Assert-WorkspacePath (Join-Path $unicodePath 'beta.bin')), [byte[]]::new(1536))
    } else {
        $ScanPath = (Resolve-Path -LiteralPath $ScanPath).ProviderPath
        if (-not (Test-Path -LiteralPath $ScanPath -PathType Container)) { throw 'ScanPath must identify a directory.' }
    }

    $hardwarePath = Assert-WorkspacePath (Join-Path $runPath 'hardware.json')
    $jsonPath = Assert-WorkspacePath (Join-Path $runPath 'scan.json')
    $csvPath = Assert-WorkspacePath (Join-Path $runPath 'scan.csv')
    Invoke-Application @('--headless', '--report', $hardwarePath, '--scan', $ScanPath, '--export', $jsonPath)
    Invoke-Application @('--headless', '--scan', $ScanPath, '--export', $csvPath)
    $hardware = Get-Content -LiteralPath $hardwarePath -Raw | ConvertFrom-Json
    $scan = Get-Content -LiteralPath $jsonPath -Raw | ConvertFrom-Json
    $csv = @(Import-Csv -LiteralPath $csvPath)
    if ($hardware.schema_version -ne 1 -or $null -eq $hardware.sample -or $null -eq $hardware.inventory) { throw 'Hardware report schema check failed.' }
    if ($scan.schema_version -ne 1 -or @($scan.nodes).Count -eq 0 -or $csv.Count -eq 0) { throw 'Storage export schema/content check failed.' }
    if ($fixture) {
        $rootNode = @($scan.nodes | Where-Object { $null -eq $_.parent })
        $csvRoot = @($csv | Where-Object { [string]::IsNullOrEmpty($_.parent_id) })
        if ($scan.status -ne 'complete' -or $scan.summary.errors -ne 0) { throw 'The controlled storage fixture did not complete without errors.' }
        if ($rootNode.Count -ne 1 -or $rootNode[0].logical -ne 5632 -or $rootNode[0].files -ne 2) { throw 'JSON fixture totals differ from the created files.' }
        if ($csvRoot.Count -ne 1 -or [long]$csvRoot[0].logical_bytes -ne 5632 -or [long]$csvRoot[0].file_count -ne 2) { throw 'CSV fixture totals differ from the created files.' }
        if ($csv.Count -ne @($scan.nodes).Count) { throw 'CSV and JSON fixture node counts differ.' }
    }

    if ($Capture) {
        foreach ($mode in @(@('Dark', '1.0'), @('Light', '1.5'))) {
            $capturePath = Assert-WorkspacePath (Join-Path $runPath ('screenshots-' + $mode[0].ToLowerInvariant()))
            Invoke-Application @('--capture', $capturePath, '--scan', $ScanPath, '--theme', $mode[0], '--scale', $mode[1])
            foreach ($name in @('00-Overview.png', '01-Cpu.png', '02-Gpu.png', '03-Storage.png', '04-Diagnostics.png')) {
                $imagePath = Assert-WorkspacePath (Join-Path $capturePath $name)
                if (-not (Test-Path -LiteralPath $imagePath -PathType Leaf) -or (Get-Item -LiteralPath $imagePath).Length -eq 0) { throw "Capture missing: $imagePath" }
            }
        }
    }

    $summary = [ordered]@{
        verified_at_utc = [DateTime]::UtcNow.ToString('o')
        executable = [IO.Path]::GetRelativePath($workspacePath, $binaryPath)
        executable_sha256 = (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
        tests_run = -not [bool]$SkipTests
        release_build_run = -not [bool]$SkipBuild
        controlled_fixture = $fixture
        scan_status = $scan.status
        launcher = $Launcher
        cpu_vendor = @($hardware.inventory.cpu | Where-Object label -eq 'Vendor' | ForEach-Object value) | Select-Object -First 1
        cpu_model = @($hardware.inventory.cpu | Where-Object label -eq 'Model' | ForEach-Object value) | Select-Object -First 1
        cpu_signature = @($hardware.inventory.cpu | Where-Object label -eq 'Family / model / stepping' | ForEach-Object value) | Select-Object -First 1
        cpu_instruction_sets = @(@($hardware.inventory.cpu | Where-Object label -eq 'Instruction sets (hardware)' | ForEach-Object value) | Select-Object -First 1) -split ' · ' | Where-Object { $_ }
        cpu_caches = @($hardware.inventory.cpu | Where-Object label -match '^L\d ' | ForEach-Object label)
        cpu_state = $hardware.sample.cpu_usage.state
        adapter_count = @($hardware.inventory.adapters).Count
        captures_created = [bool]$Capture
        limitation = 'Schema, fixture and capture-file checks do not establish sensor accuracy, visual quality, keyboard accessibility or all filesystem/provider support.'
    }
    $summaryPath = Assert-WorkspacePath (Join-Path $runPath 'verification.json')
    [IO.File]::WriteAllText($summaryPath, ($summary | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
    Write-Output "Verification outputs: $runPath"
    Write-Output "Storage status: $($scan.status); CPU state: $($hardware.sample.cpu_usage.state)"
    if ($Capture) { Write-Output 'Capture files created. Review all images manually; this is not a visual/accessibility pass.' }
} finally {
    Pop-Location
}
