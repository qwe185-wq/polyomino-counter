param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [ValidateRange(1, 6)][int]$N = 5,
    [ValidateRange(1, 30)][int]$Runs = 9,
    [ValidateRange(16, 65536)][int]$MemoryMiB = 512,
    [ValidateRange(1, 3600)][int]$TimeoutSeconds = 30,
    [ValidateRange(1, 256)][int]$Threads = 32,
    [string]$OutputDir = (Join-Path $PSScriptRoot "../tmp/jensen-evidence/runner/$(Get-Date -Format yyyyMMdd-HHmmss)-$([Guid]::NewGuid().ToString('N').Substring(0, 8))")
)

$ErrorActionPreference = 'Stop'
if (-not $IsWindows -and $PSVersionTable.PSEdition -eq 'Core') { throw '此启动器需要 Windows' }
if (-not [Environment]::Is64BitProcess) { throw '此启动器需要 64 位 PowerShell' }
$previousThreads = $env:RAYON_NUM_THREADS
$Exe = (Resolve-Path -LiteralPath $Exe).Path
if ([IO.Path]::GetExtension($Exe) -ne '.exe') { throw 'Exe 必须为 Windows 可执行文件' }
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
[IO.Directory]::CreateDirectory($OutputDir) | Out-Null
if (@(Get-ChildItem -LiteralPath $OutputDir -Force).Count -ne 0) { throw "输出目录已有内容，拒绝覆盖：$OutputDir" }
# CreateNew 在并发启动时也只允许一个运行占用该目录。
$lock = [IO.File]::Open((Join-Path $OutputDir '.run-lock'), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
try {
Add-Type -Path (Join-Path $PSScriptRoot 'BoundedProcess.cs') -ErrorAction Stop

# one-sided polyomino 的已知三分类结果；n=6 来自本任务已验证的基线。
$expected = @(
    @('1', '1', '0'),
    @('4', '4', '0'),
    @('46', '44', '2'),
    @('2404', '1899', '505'),
    @('520818', '267976', '252842'),
    @('410964612', '112877832', '298086780')
)
function Get-Median([double[]]$Values) {
    $sortedValues = @($Values | Sort-Object)
    $middle = [int][Math]::Floor($sortedValues.Count / 2)
    if ($sortedValues.Count % 2 -eq 1) { return [double]$sortedValues[$middle] }
    return (([double]$sortedValues[$middle - 1] + [double]$sortedValues[$middle]) / 2.0)
}

$env:RAYON_NUM_THREADS = [string]$Threads
$rows = [Collections.Generic.List[object]]::new()
$exeHash = (Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash
$metadata = [ordered]@{
    executable = $Exe
    sha256 = $exeHash
    n = $N
    runs = $Runs
    memory_limit_bytes = [long]$MemoryMiB * 1048576L
    timeout_seconds = $TimeoutSeconds
    rayon_threads = $Threads
    process_seconds_definition = 'CreateProcess 调用前至子进程退出；包含进程创建、Job 挂载和执行，不含脚本编译或日志处理'
    argv = @([string]$N, '--algorithm', 'transfer')
}
$metadata | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $OutputDir 'metadata.json') -Encoding utf8

    for ($run = 1; $run -le $Runs; $run++) {
        $stdoutPath = Join-Path $OutputDir "run-$run.stdout.log"
        $stderrPath = Join-Path $OutputDir "run-$run.stderr.log"
        $result = [BoundedProcess]::Run($Exe, @([string]$N, '--algorithm', 'transfer'),
            $stdoutPath, $stderrPath, ([ulong]$MemoryMiB * [ulong]1048576),
            [uint32]($TimeoutSeconds * 1000))
        $stdoutText = [string](Get-Content -LiteralPath $stdoutPath -Raw -Encoding utf8)
        $timing = [regex]::Match($stdoutText, '算法耗时:\s*([0-9.]+)s')
        $countMatches = [regex]::Matches($stdoutText, '(?m)^n=([0-9]+): total=([0-9]+), no_hole=([0-9]+), has_hole=([0-9]+)\r?$')
        $actual = @{}
        $countsValid = ($countMatches.Count -eq $N)
        foreach ($match in $countMatches) {
            $level = [int]$match.Groups[1].Value
            if ($level -lt 1 -or $level -gt $N -or $actual.ContainsKey($level)) { $countsValid = $false; continue }
            $actual[$level] = @($match.Groups[2].Value, $match.Groups[3].Value, $match.Groups[4].Value)
        }
        for ($level = 1; $level -le $N; $level++) {
            if (-not $actual.ContainsKey($level)) { $countsValid = $false; continue }
            for ($part = 0; $part -lt 3; $part++) {
                if ($actual[$level][$part] -ne $expected[$level - 1][$part]) { $countsValid = $false }
            }
        }
        $row = [pscustomobject]@{
            run = $run
            n = $N
            argv = @([string]$N, '--algorithm', 'transfer')
            compute_seconds = $(if ($timing.Success) { [double]::Parse($timing.Groups[1].Value, [Globalization.CultureInfo]::InvariantCulture) } else { $null })
            process_seconds = $result.ElapsedSeconds
            count = $(if ($actual.ContainsKey($N)) { $actual[$N][0] } else { $null })
            no_hole = $(if ($actual.ContainsKey($N)) { $actual[$N][1] } else { $null })
            has_hole = $(if ($actual.ContainsKey($N)) { $actual[$N][2] } else { $null })
            counts_valid = $countsValid
            peak_job_memory_bytes = $result.PeakJobMemoryBytes
            peak_process_memory_bytes = $result.PeakProcessMemoryBytes
            # 硬限制会拒绝内存提交；Windows 完成端口通知可能丢失，false 仅表示未观察到。
            memory_limit_event = $result.MemoryLimitEvent
            memory_limit_termination = ($result.MemoryLimitEvent -and $result.ExitCode -ne 0)
            memory_limit_status = $(if ($result.MemoryLimitEvent -and $result.ExitCode -ne 0) { 'observed_failed_run' }
                elseif ($result.MemoryLimitEvent) { 'observed_event' }
                elseif ($result.ExitCode -ne 0) { 'unknown_on_failed_run' }
                else { 'not_observed' })
            timed_out = $result.TimedOut
            exit_code = $result.ExitCode
            stdout_file = $stdoutPath
            stderr_file = $stderrPath
        }
        $rows.Add($row)
        ConvertTo-Json -InputObject @($rows.ToArray()) -Depth 5 | Set-Content -LiteralPath (Join-Path $OutputDir 'results.json') -Encoding utf8
        if ($result.TimedOut) { throw "第 $run 次计数超时（${TimeoutSeconds}s）；证据：$OutputDir" }
        if ($result.MemoryLimitEvent) { throw "第 $run 次计数触及内存限制；证据：$OutputDir" }
        if ($result.ExitCode -ne 0) { throw "第 $run 次计数失败，exit=$($result.ExitCode)；证据：$OutputDir" }
        if (-not $timing.Success) { throw "第 $run 次计数缺少算法耗时；证据：$OutputDir" }
        if (-not $countsValid) { throw "第 $run 次计数缺失或三分类结果错误；证据：$OutputDir" }
        Write-Host ("run {0}/{1}: count={2}, compute={3:N3}s, process={4:N3}s, peak_job={5:N1} MiB" -f
            $run, $Runs, $row.count, $row.compute_seconds, $row.process_seconds,
            ($row.peak_job_memory_bytes / 1MB))
    }
    $sorted = @($rows.compute_seconds | Sort-Object)
    $summary = [pscustomobject]@{
        n = $N
        runs = $Runs
        count = $rows[0].count
        no_hole = $rows[0].no_hole
        has_hole = $rows[0].has_hole
        compute_median_seconds = Get-Median -Values ([double[]]@($rows.compute_seconds))
        process_median_seconds = Get-Median -Values ([double[]]@($rows.process_seconds))
        compute_min_seconds = $sorted[0]
        compute_max_seconds = $sorted[-1]
        peak_job_memory_bytes = ($rows.peak_job_memory_bytes | Measure-Object -Maximum).Maximum
        peak_process_memory_bytes = ($rows.peak_process_memory_bytes | Measure-Object -Maximum).Maximum
    }
    $summary | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $OutputDir 'summary.json') -Encoding utf8
    $summary | Format-List
    Write-Output "Evidence: $OutputDir"
} finally {
    $env:RAYON_NUM_THREADS = $previousThreads
    $lock.Dispose()
}
