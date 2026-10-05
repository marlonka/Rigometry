#Requires -Version 7.0
[CmdletBinding()]
param(
    [ValidateRange(1, 60)][int]$WarmupSeconds = 5,
    [ValidateRange(1, 120)][int]$SampleSeconds = 10,
    [ValidateRange(10, 3600)][int]$ScanTimeoutSeconds = 120
)

# Run after cargo build --release --locked. This script does not build or test.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Native benchmarking requires Windows.' }
$workspacePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$workspacePrefix = $workspacePath.TrimEnd('\') + '\'

function Assert-WorkspacePath([string]$Path) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    if ($fullPath -ne $workspacePath -and -not $fullPath.StartsWith($workspacePrefix, [StringComparison]::OrdinalIgnoreCase)) { throw "Path escapes the workspace: $fullPath" }
    $checkPath = $fullPath
    while ($checkPath -and $checkPath -ne $workspacePath) {
        if (Test-Path -LiteralPath $checkPath) {
            if ((Get-Item -LiteralPath $checkPath -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Reparse path refused: $checkPath" }
        }
        $checkPath = [IO.Path]::GetDirectoryName($checkPath)
    }
    return $fullPath
}

function Start-OwnedProcess([string]$LogName, [string[]]$ApplicationArguments) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $binaryPath
    $start.WorkingDirectory = $workspacePath
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $ApplicationArguments) { $start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::Start($start)
    return [pscustomobject]@{
        Process = $process
        Stdout = $process.StandardOutput.ReadToEndAsync()
        Stderr = $process.StandardError.ReadToEndAsync()
        LogName = $LogName
    }
}

function Finish-OwnedProcess($Owned) {
    $forced = $false
    try {
        if (-not $Owned.Process.HasExited) {
            $closed = $Owned.Process.CloseMainWindow()
            if (-not $closed -or -not $Owned.Process.WaitForExit(3000)) {
                $Owned.Process.Kill($true)
                $forced = $true
                [void]$Owned.Process.WaitForExit(5000)
            }
        }
        foreach ($stream in @(@('stdout', $Owned.Stdout), @('stderr', $Owned.Stderr))) {
            [void]$stream[1].Wait(1000)
            $text = if ($stream[1].IsCompletedSuccessfully) { $stream[1].Result } else { 'Log stream did not close within the cleanup timeout.' }
            $path = Assert-WorkspacePath (Join-Path $runPath ($Owned.LogName + '.' + $stream[0] + '.log'))
            [IO.File]::WriteAllText($path, $text, [Text.UTF8Encoding]::new($false))
        }
    } finally {
        $Owned.Process.Dispose()
    }
    return $forced
}

function Wait-OwnedProcess($Process, [double]$Seconds) {
    $wait = [Diagnostics.Stopwatch]::StartNew()
    while ($wait.Elapsed.TotalSeconds -lt $Seconds) {
        if ($Process.HasExited) { throw "Benchmark process exited unexpectedly with code $($Process.ExitCode)." }
        # No individual wait exceeds one second.
        Start-Sleep -Milliseconds ([int][Math]::Max(1, [Math]::Min(1000, ($Seconds - $wait.Elapsed.TotalSeconds) * 1000)))
    }
}

$binaryPath = Assert-WorkspacePath (Join-Path $workspacePath 'target/release/rigometry.exe')
if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) { throw 'Release executable missing. Run cargo build --release --locked first.' }
$runName = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$runPath = Assert-WorkspacePath (Join-Path $workspacePath "artifacts/benchmarks/$runName")
[IO.Directory]::CreateDirectory($runPath) | Out-Null
Write-Output "Benchmark outputs: $runPath"

$fixturePath = Assert-WorkspacePath (Join-Path $workspacePath 'artifacts/benchmark-fixture')
$createdFixture = -not (Test-Path -LiteralPath $fixturePath)
if ($createdFixture) {
    [IO.Directory]::CreateDirectory($fixturePath) | Out-Null
    $content = [byte[]]::new(1024)
    for ($folderIndex = 0; $folderIndex -lt 50; $folderIndex++) {
        $folderPath = Assert-WorkspacePath (Join-Path $fixturePath ('folder-{0:D2}' -f $folderIndex))
        [IO.Directory]::CreateDirectory($folderPath) | Out-Null
        for ($fileIndex = 0; $fileIndex -lt 200; $fileIndex++) {
            $filePath = Assert-WorkspacePath (Join-Path $folderPath ('file-{0:D3}.bin' -f $fileIndex))
            [IO.File]::WriteAllBytes($filePath, $content)
        }
    }
}

# Validate without modifying an existing fixture. Metadata enumeration warms the
# filesystem cache; results must not be described as cold-cache performance.
$folders = @(Get-ChildItem -LiteralPath $fixturePath -Directory)
if ($folders.Count -ne 50 -or @(Get-ChildItem -LiteralPath $fixturePath -File).Count -ne 0) { throw 'Fixture must contain exactly 50 folders and no root files. Existing data was not changed.' }
[long]$fixtureFiles = 0
[long]$fixtureBytes = 0
foreach ($folder in $folders) {
    $folderPath = Assert-WorkspacePath $folder.FullName
    if (@(Get-ChildItem -LiteralPath $folderPath -Directory).Count -ne 0) { throw 'Fixture contains unexpected nested directories.' }
    $files = @(Get-ChildItem -LiteralPath $folderPath -File)
    if ($files.Count -ne 200) { throw "Fixture folder has $($files.Count) files; expected 200." }
    foreach ($file in $files) {
        # Files came from a nonrecursive enumeration of the validated folder.
        if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Fixture contains a reparse file.' }
        if ($file.Length -ne 1024) { throw 'Fixture contains a file whose length differs from 1024 bytes.' }
        $fixtureFiles++
        $fixtureBytes += $file.Length
    }
}
if ($fixtureFiles -ne 10000 -or $fixtureBytes -ne 10240000) { throw 'Fixture verification failed.' }

$logicalProcessors = [Environment]::ProcessorCount
$startupWatch = [Diagnostics.Stopwatch]::StartNew()
$gui = Start-OwnedProcess 'gui' @()
$guiResult = $null
try {
    $inputIdle = $false
    $inputIdleError = $null
    try { $inputIdle = $gui.Process.WaitForInputIdle(10000) } catch { $inputIdleError = $_.Exception.Message }
    $startupWatch.Stop()
    $startupMilliseconds = $startupWatch.Elapsed.TotalMilliseconds
    if ($gui.Process.HasExited) { throw "GUI startup failed with exit code $($gui.Process.ExitCode)." }
    Wait-OwnedProcess $gui.Process $WarmupSeconds
    $gui.Process.Refresh()
    $warmWorking = $gui.Process.WorkingSet64
    $warmPrivate = $gui.Process.PrivateMemorySize64
    $startCpu = $gui.Process.TotalProcessorTime.TotalSeconds
    $sampleWatch = [Diagnostics.Stopwatch]::StartNew()
    $peakSampledWorking = $warmWorking
    $peakSampledPrivate = $warmPrivate
    while ($sampleWatch.Elapsed.TotalSeconds -lt $SampleSeconds) {
        Wait-OwnedProcess $gui.Process ([Math]::Min(1, $SampleSeconds - $sampleWatch.Elapsed.TotalSeconds))
        $gui.Process.Refresh()
        $peakSampledWorking = [Math]::Max($peakSampledWorking, $gui.Process.WorkingSet64)
        $peakSampledPrivate = [Math]::Max($peakSampledPrivate, $gui.Process.PrivateMemorySize64)
    }
    $gui.Process.Refresh()
    $cpuSeconds = $gui.Process.TotalProcessorTime.TotalSeconds - $startCpu
    $sampleWatch.Stop()
    $elapsedSeconds = $sampleWatch.Elapsed.TotalSeconds
    if ($cpuSeconds -lt 0 -or $elapsedSeconds -le 0 -or $logicalProcessors -le 0) { throw 'Invalid process CPU measurement.' }
    $oneCorePercent = 100 * $cpuSeconds / $elapsedSeconds
    $guiResult = [ordered]@{
        started_pid = $gui.Process.Id
        launch_arguments = @()
        launch_window_style = 'Hidden'
        startup_proxy = 'Process launch to WaitForInputIdle; not first paint, hardware readiness or user-visible launch latency'
        input_idle_reached = $inputIdle
        input_idle_error = $inputIdleError
        startup_proxy_ms = [Math]::Round($startupMilliseconds, 3)
        warmup_seconds = $WarmupSeconds
        measurement_seconds = [Math]::Round($elapsedSeconds, 6)
        process_cpu_seconds = [Math]::Round($cpuSeconds, 6)
        cpu_percent_one_logical_processor = [Math]::Round($oneCorePercent, 4)
        cpu_percent_machine = [Math]::Round($oneCorePercent / $logicalProcessors, 4)
        logical_processors_for_normalization = $logicalProcessors
        warm_working_set_mib = [Math]::Round($warmWorking / 1MB, 3)
        warm_private_memory_mib = [Math]::Round($warmPrivate / 1MB, 3)
        end_working_set_mib = [Math]::Round($gui.Process.WorkingSet64 / 1MB, 3)
        end_private_memory_mib = [Math]::Round($gui.Process.PrivateMemorySize64 / 1MB, 3)
        peak_polled_working_set_mib = [Math]::Round($peakSampledWorking / 1MB, 3)
        peak_polled_private_memory_mib = [Math]::Round($peakSampledPrivate / 1MB, 3)
        conditions = 'No automated desktop input. Default application launch and persisted preferences; active page is not inferred. Background system activity is uncontrolled.'
    }
} finally {
    $forced = Finish-OwnedProcess $gui
    if ($null -ne $guiResult) { $guiResult['forced_own_process_cleanup'] = $forced }
}

$exportPath = Assert-WorkspacePath (Join-Path $runPath 'scan.json')
$scanWatch = [Diagnostics.Stopwatch]::StartNew()
$scanProcess = Start-OwnedProcess 'scan' @('--headless', '--scan', $fixturePath, '--export', $exportPath)
$scanExitCode = $null
try {
    while (-not $scanProcess.Process.WaitForExit(1000)) {
        if ($scanWatch.Elapsed.TotalSeconds -gt $ScanTimeoutSeconds) { throw "Headless scan exceeded $ScanTimeoutSeconds seconds." }
    }
    $scanWatch.Stop()
    $scanExitCode = $scanProcess.Process.ExitCode
    if ($scanExitCode -ne 0) { throw "Headless scan exited with code $scanExitCode." }
} finally {
    [void](Finish-OwnedProcess $scanProcess)
}
$scan = Get-Content -LiteralPath $exportPath -Raw | ConvertFrom-Json
$rootNodes = @($scan.nodes | Where-Object { $null -eq $_.parent })
if ($scan.schema_version -ne 1 -or $scan.status -ne 'complete' -or $scan.summary.errors -ne 0 -or $scan.summary.cancelled) { throw 'Benchmark scan did not complete cleanly.' }
if ($rootNodes.Count -ne 1 -or $rootNodes[0].files -ne 10000 -or $rootNodes[0].logical -ne 10240000) { throw 'Benchmark scan totals differ from the independently verified fixture.' }
if (@($scan.nodes).Count -ne 10051) { throw 'Benchmark scan node count differs from 10000 files + 50 folders + root.' }

$result = [ordered]@{
    measured_at_utc = [DateTime]::UtcNow.ToString('o')
    executable = [IO.Path]::GetRelativePath($workspacePath, $binaryPath)
    executable_sha256 = (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    os = [Runtime.InteropServices.RuntimeInformation]::OSDescription
    architecture = [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
    gui = $guiResult
    storage = [ordered]@{
        fixture_created = $createdFixture
        fixture_relative_path = [IO.Path]::GetRelativePath($workspacePath, $fixturePath)
        files = $fixtureFiles
        folders_excluding_root = 50
        bytes_per_file = 1024
        logical_bytes = $fixtureBytes
        reported_files = $rootNodes[0].files
        reported_logical_bytes = $rootNodes[0].logical
        reported_allocated_bytes = $rootNodes[0].allocated
        reported_node_count = @($scan.nodes).Count
        scanner_elapsed_ms = $scan.summary.elapsed_ms
        process_and_export_wall_ms = [Math]::Round($scanWatch.Elapsed.TotalMilliseconds, 3)
        method = $scan.summary.method
        cache_condition = 'Metadata validated before measurement; cache not cleared. Warm-cache observation, not a cold-disk benchmark.'
        status = $scan.status
        exit_code = $scanExitCode
    }
}
$resultPath = Assert-WorkspacePath (Join-Path $runPath 'benchmark.json')
[IO.File]::WriteAllText($resultPath, ($result | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
Write-Output "Startup proxy: $($guiResult.startup_proxy_ms) ms (input idle: $($guiResult.input_idle_reached))"
Write-Output "Idle CPU: $($guiResult.cpu_percent_one_logical_processor)% of one logical processor; $($guiResult.cpu_percent_machine)% machine-normalized"
Write-Output "After warmup: $($guiResult.warm_working_set_mib) MiB working set; $($guiResult.warm_private_memory_mib) MiB private memory"
Write-Output "10000-file scan: $($scan.summary.elapsed_ms) ms scanner; $([Math]::Round($scanWatch.Elapsed.TotalMilliseconds, 3)) ms including process/export"
Write-Output "Result: $resultPath"
