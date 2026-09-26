param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [ValidateRange(1, 6)][int]$N = 5,
    [ValidateRange(1, 9)][int]$Runs = 1,
    [ValidateRange(16, 65536)][int]$MemoryMiB = 128,
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 60,
    [ValidateRange(1, 256)][int]$Threads = 8,
    [ValidateSet('frontier', 'canonical', 'bfs', 'redelmeier')][string]$Algorithm = 'frontier',
    [ValidateSet('raw', 'zip')][string]$Mode = 'raw',
    [ValidateSet('native', '7z')][string]$CompressionBackend = 'native',
    [ValidateRange(0, 9)][int]$CompressionLevel = 1,
    [string]$OutputDir = (Join-Path $PSScriptRoot "../tmp/export-evidence/$(Get-Date -Format yyyyMMdd-HHmmss)-$([Guid]::NewGuid().ToString('N').Substring(0, 8))")
)

$ErrorActionPreference = 'Stop'
if (-not $IsWindows -and $PSVersionTable.PSEdition -eq 'Core') { throw '此启动器需要 Windows' }
if (-not [Environment]::Is64BitProcess) { throw '此启动器需要 64 位 PowerShell' }
$Exe = (Resolve-Path -LiteralPath $Exe).Path
if (-not [IO.File]::Exists($Exe) -or [IO.Path]::GetExtension($Exe) -ne '.exe') { throw 'Exe 必须为现存 Windows 可执行文件' }
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
[IO.Directory]::CreateDirectory($OutputDir) | Out-Null
if (@(Get-ChildItem -LiteralPath $OutputDir -Force).Count -ne 0) { throw "输出目录已有内容，拒绝覆盖：$OutputDir" }
$lock = [IO.File]::Open((Join-Path $OutputDir '.run-lock'), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
$previousThreads = $env:RAYON_NUM_THREADS

# 累计 one-sided 三分类；本脚本校验计数和物理长度，不证明形状集合相等。
$expected = @(
    @(1L, 1L, 0L), @(4L, 4L, 0L), @(46L, 44L, 2L),
    @(2404L, 1899L, 505L), @(520818L, 267976L, 252842L),
    @(410964612L, 112877832L, 298086780L)
)
$chunkBytes = 80000000L

function Read-Seconds([string]$Text, [string]$Label) {
    $matches = [regex]::Matches($Text, [regex]::Escape($Label) + ':\s*([0-9]+(?:\.[0-9]+)?)s')
    if ($matches.Count -ne 1) { throw "计时字段缺失或重复：$Label" }
    return [double]::Parse($matches[0].Groups[1].Value, [Globalization.CultureInfo]::InvariantCulture)
}

function Assert-ChunkLengths([object[]]$Items, [long]$ExpectedCount) {
    $expectedChunks = [long][Math]::Ceiling($ExpectedCount / 10000000.0)
    if ($Items.Count -ne $expectedChunks) { throw "chunk 数不符：$($Items.Count) != $expectedChunks" }
    $bytes = 0L
    for ($i = 0; $i -lt $Items.Count; $i++) {
        $item = $Items[$i]
        $number = $i + 1
        if ($item.Name -ne ('shapes_{0:D6}.bin' -f $number)) { throw "chunk 名称或顺序错误：$($item.Name)" }
        $length = [long]$item.Length
        if ($length -le 0 -or $length -gt $chunkBytes -or $length % 8 -ne 0) { throw "chunk 长度错误：$($item.Name) $length" }
        if ($number -lt $Items.Count -and $length -ne $chunkBytes) { throw "非末块长度错误：$($item.Name) $length" }
        $bytes += $length
    }
    if ($bytes -ne $ExpectedCount * 8L) { throw "流字节数不符：$bytes != $($ExpectedCount * 8L)" }
    return $bytes
}

function Get-StreamBytes([string]$Path, [string]$Mode, [long]$ExpectedCount) {
    if ($Mode -eq 'raw') {
        if (-not [IO.Directory]::Exists($Path)) { throw "缺少 raw 流：$Path" }
        if (@(Get-ChildItem -LiteralPath $Path -Directory -Force).Count -ne 0) { throw "raw 流有子目录：$Path" }
        $items = @(Get-ChildItem -LiteralPath $Path -File -Force | Sort-Object Name | ForEach-Object {
            [pscustomobject]@{ Name = $_.Name; Length = $_.Length }
        })
        return Assert-ChunkLengths $items $ExpectedCount
    }
    if (-not [IO.File]::Exists($Path)) { throw "缺少 ZIP 流：$Path" }
    $archive = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $directoryName = [IO.Path]::GetFileNameWithoutExtension($Path)
        $items = @($archive.Entries | Where-Object { $_.Name -ne '' } | ForEach-Object {
            if (-not $_.FullName.StartsWith("$directoryName/", [StringComparison]::Ordinal)) { throw "ZIP 条目路径错误：$($_.FullName)" }
            [pscustomobject]@{ Name = $_.Name; Length = $_.Length }
        } | Sort-Object Name)
        return Assert-ChunkLengths $items $ExpectedCount
    } finally {
        $archive.Dispose()
    }
}

function Test-Dataset([string]$Dir, [int]$Maximum, [string]$StorageMode) {
    $manifestPath = Join-Path $Dir 'dataset.json'
    if (-not [IO.File]::Exists($manifestPath)) { throw "缺少 dataset.json：$Dir" }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding utf8 | ConvertFrom-Json
    $last = $expected[$Maximum - 1]
    if ($manifest.complete -ne $true -or $manifest.format_version -ne 2 -or $manifest.max_n -ne $Maximum -or
        [long]$manifest.count -ne [long]$last[0] -or $manifest.storage_layout -ne 'classified-single-copy' -or
        $manifest.encoding -ne 'u64-le-stride8') { throw 'manifest 完成状态、版本、n、计数或格式错误' }
    $required = @{}
    for ($md = 1; $md -le $Maximum; $md++) {
        $beforeNo = if ($md -eq 1) { 0L } else { [long]$expected[$md - 2][1] }
        $beforeHole = if ($md -eq 1) { 0L } else { [long]$expected[$md - 2][2] }
        foreach ($category in @(@('no_holes', $false, ([long]$expected[$md - 1][1] - $beforeNo)),
                @('with_holes', $true, ([long]$expected[$md - 1][2] - $beforeHole)))) {
            if ($category[2] -eq 0) { continue }
            $extension = if ($StorageMode -eq 'zip') { '.zip' } else { '' }
            $relative = '{0}/n{1:D2}_fixed{2}' -f $category[0], $md, $extension
            $required[$relative] = [pscustomobject]@{ Count = [long]$category[2]; Dimension = $md; Hole = [bool]$category[1] }
        }
    }
    # 数据集是物理单份：拒绝all副本、越界分类和未列出的文件/目录。
    foreach ($entry in @(Get-ChildItem -LiteralPath $Dir -Force)) {
        if ($entry.Name -eq 'dataset.json' -and -not $entry.PSIsContainer) { continue }
        if ($entry.Name -notin @('no_holes', 'with_holes') -or -not $entry.PSIsContainer) { throw "数据集含未列出条目：$($entry.FullName)" }
        foreach ($streamEntry in @(Get-ChildItem -LiteralPath $entry.FullName -Force)) {
            $streamPath = "$($entry.Name)/$($streamEntry.Name)"
            if (-not $required.ContainsKey($streamPath) -or $streamEntry.PSIsContainer -ne ($StorageMode -eq 'raw')) { throw "数据集含未列出分类流：$($streamEntry.FullName)" }
        }
    }
    if (@($manifest.streams).Count -ne $required.Count) { throw 'manifest 分类流数量不符' }
    $seen = @{}
    $totalBytes = 0L
    foreach ($stream in @($manifest.streams)) {
        $relative = [string]$stream.path
        if (-not $required.ContainsKey($relative) -or $seen.ContainsKey($relative)) { throw "manifest 多余或重复流：$relative" }
        $seen[$relative] = $true
        $want = $required[$relative]
        if ([long]$stream.count -ne $want.Count -or [int]$stream.max_dimension -ne $want.Dimension -or
            [bool]$stream.has_hole -ne $want.Hole) { throw "manifest 分类流计数或属性错误：$relative" }
        $relativeNative = $relative.Replace('/', [IO.Path]::DirectorySeparatorChar)
        $totalBytes += Get-StreamBytes (Join-Path $Dir $relativeNative) $StorageMode $want.Count
    }
    if ($totalBytes -ne [long]$last[0] * 8L) { throw "数据集原始记录总字节数错误：$totalBytes" }
    return [pscustomobject]@{
        dataset_id = [string]$manifest.dataset_id
        count = [long]$manifest.count
        stream_count = $required.Count
        raw_record_bytes = $totalBytes
        manifest_sha256 = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash
    }
}

try {
    Add-Type -Path (Join-Path $PSScriptRoot 'BoundedProcess.cs') -ErrorAction Stop
    if ($Mode -eq 'zip') {
        Add-Type -AssemblyName System.IO.Compression -ErrorAction Stop
        if (-not ('System.IO.Compression.ZipFile' -as [type])) {
            Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction Stop
        }
    }
    $env:RAYON_NUM_THREADS = [string]$Threads
    $argvBase = @([string]$N, '--algorithm', $Algorithm, '--export', '--export-dir')
    if ($Mode -eq 'raw') { $argvTail = @('--no-compress') }
    else { $argvTail = @('--compression-backend', $CompressionBackend, '--compression-level', [string]$CompressionLevel) }
    $metadata = [ordered]@{
        executable = $Exe
        executable_sha256 = (Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash
        script_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash
        n = $N; runs = $Runs; algorithm = $Algorithm; mode = $Mode
        compression_backend = $CompressionBackend; compression_level = $CompressionLevel
        rayon_threads = $Threads; memory_limit_bytes = [long]$MemoryMiB * 1048576L
        timeout_seconds = $TimeoutSeconds
        process_seconds_definition = 'CreateProcess 前至 Job 子进程退出；包含算法、写盘、ZIP 收尾/外部压缩和进程开销'
        algorithm_seconds_definition = '程序打印算法耗时；native ZIP 流式压缩包含在此项，ZIP 收尾另计'
        raw_record_bytes_definition = 'raw 文件长度或 ZIP 条目未压缩长度之和；只验证分类流数目、计数及字节数，不证明集合相等'
    }
    $metadata | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $OutputDir 'metadata.json') -Encoding utf8
    $rows = [Collections.Generic.List[object]]::new()
    for ($run = 1; $run -le $Runs; $run++) {
        $datasetDir = Join-Path $OutputDir ('dataset-{0:D2}' -f $run)
        if (Test-Path -LiteralPath $datasetDir) { throw "数据集目录已存在：$datasetDir" }
        $stdoutPath = Join-Path $OutputDir ('run-{0:D2}.stdout.log' -f $run)
        $stderrPath = Join-Path $OutputDir ('run-{0:D2}.stderr.log' -f $run)
        $argv = @($argvBase + @($datasetDir) + $argvTail)
        $row = [ordered]@{ run = $run; argv = $argv; dataset_dir = $datasetDir; stdout_file = $stdoutPath; stderr_file = $stderrPath; valid = $false; error = $null }
        try {
            $result = [BoundedProcess]::Run($Exe, [string[]]$argv, $stdoutPath, $stderrPath,
                ([ulong]$MemoryMiB * [ulong]1048576), [uint32]($TimeoutSeconds * 1000))
            $row.process_seconds = $result.ElapsedSeconds
            $row.exit_code = $result.ExitCode
            $row.timed_out = $result.TimedOut
            $row.memory_limit_event = $result.MemoryLimitEvent
            $row.memory_limit_status = if ($result.MemoryLimitEvent -and $result.ExitCode -ne 0) { 'observed_failed_run' }
                elseif ($result.MemoryLimitEvent) { 'observed_event' }
                elseif ($result.ExitCode -ne 0) { 'unknown_on_failed_run' }
                else { 'not_observed' }
            $row.peak_job_memory_bytes = $result.PeakJobMemoryBytes
            $row.peak_process_memory_bytes = $result.PeakProcessMemoryBytes
            if ($result.TimedOut) { throw 'Job 超时' }
            if ($result.MemoryLimitEvent) { throw '观察到 Job 内存限制事件' }
            if ($result.ExitCode -ne 0) { throw "程序退出码 $($result.ExitCode)" }
            $stdoutText = [string](Get-Content -LiteralPath $stdoutPath -Raw -Encoding utf8)
            $countMatches = [regex]::Matches($stdoutText, '(?m)^n=([0-9]+): total=([0-9]+), no_hole=([0-9]+), has_hole=([0-9]+)\r?$')
            if ($countMatches.Count -ne $N) { throw '控制台 n 行数不符' }
            $found = @{}
            foreach ($match in $countMatches) {
                $level = [int]$match.Groups[1].Value
                if ($level -lt 1 -or $level -gt $N -or $found.ContainsKey($level)) { throw '控制台 n 重复或越界' }
                $found[$level] = @([long]$match.Groups[2].Value, [long]$match.Groups[3].Value, [long]$match.Groups[4].Value)
            }
            for ($level = 1; $level -le $N; $level++) {
                if (-not $found.ContainsKey($level)) { throw "控制台缺少 n=$level" }
                for ($part = 0; $part -lt 3; $part++) {
                    if ($found[$level][$part] -ne $expected[$level - 1][$part]) { throw "n=$level 分类计数错误" }
                }
            }
            $row.algorithm_seconds = Read-Seconds $stdoutText '算法耗时'
            $row.program_total_seconds = Read-Seconds $stdoutText '总耗时'
            if ($Mode -eq 'zip' -and $CompressionBackend -eq 'native') { $row.zip_finish_seconds = Read-Seconds $stdoutText 'ZIP收尾耗时' }
            if ($Mode -eq 'zip' -and $CompressionBackend -eq '7z') { $row.external_compression_seconds = Read-Seconds $stdoutText '压缩耗时' }
            $dataset = Test-Dataset $datasetDir $N $Mode
            $row.dataset_id = $dataset.dataset_id
            $row.manifest_sha256 = $dataset.manifest_sha256
            $row.count = $dataset.count
            $row.stream_count = $dataset.stream_count
            $row.raw_record_bytes = $dataset.raw_record_bytes
            $row.valid = $true
        } catch {
            $row.error = $_.Exception.Message
        }
        $rows.Add([pscustomobject]$row)
        [pscustomobject]$row | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $OutputDir ('run-{0:D2}.result.json' -f $run)) -Encoding utf8
        ConvertTo-Json -InputObject @($rows.ToArray()) -Depth 6 | Set-Content -LiteralPath (Join-Path $OutputDir 'results.json') -Encoding utf8
        if (-not $row.valid) { throw "第 $run 次导出失败或验证未通过：$($row.error)；证据：$OutputDir" }
        Write-Host ("run {0}/{1}: count={2}, algorithm={3:N3}s, process={4:N3}s, peak_job={5:N1} MiB" -f
            $run, $Runs, $row.count, $row.algorithm_seconds, $row.process_seconds, ($row.peak_job_memory_bytes / 1MB))
    }
    Write-Output "Evidence: $OutputDir"
} finally {
    $env:RAYON_NUM_THREADS = $previousThreads
    $lock.Dispose()
}
