param(
    [Parameter(Mandatory=$true)][string]$BaselineExe,
    [string]$CandidateExe = "$PSScriptRoot/../target/release/room-count.exe",
    [ValidateRange(1,5)][int]$N = 5,
    [ValidateRange(3,100)][int]$Runs = 9,
    [string]$OutputDir = "$PSScriptRoot/../tmp/benchmark-$(Get-Date -Format yyyyMMdd-HHmmss)"
)
$ErrorActionPreference='Stop'
$BaselineExe=(Resolve-Path -LiteralPath $BaselineExe).Path
$CandidateExe=(Resolve-Path -LiteralPath $CandidateExe).Path
New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
$OutputDir=(Resolve-Path -LiteralPath $OutputDir).Path
$previousThreads=$env:RAYON_NUM_THREADS
$env:RAYON_NUM_THREADS='32'
$rows=[Collections.Generic.List[object]]::new()
function Measure-Run([string]$Name,[string]$Exe,[string[]]$RunArgs,[int]$Run) {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    $output=& $Exe @RunArgs 2>&1
    $exitCode=$LASTEXITCODE
    $watch.Stop()
    $output | Set-Content -LiteralPath "$OutputDir/$Name-$Run.log"
    if($exitCode -ne 0) {throw "Failed: $Name $Run (exit=$exitCode)"}
    $text=$output -join "`n"
    $match=[regex]::Match($text,'算法耗时: ([0-9.]+)s')
    if(-not $match.Success) {$match=[regex]::Match($text,'总耗时: ([0-9.]+)s')}
    if(-not $match.Success) {throw "Missing timing: $Name"}
    if($N -eq 5 -and $text -notmatch '520818') {throw 'Missing expected count'}
    if($Run -gt 0) {$rows.Add([pscustomobject]@{
        name=$Name;run=$Run;compute_seconds=[double]$match.Groups[1].Value;
        process_seconds=$watch.Elapsed.TotalSeconds;exit_code=$exitCode
    })}
}
try {
    for($run=0;$run -le $Runs;$run++) {
        Measure-Run 'baseline' $BaselineExe @("$N") $run
        foreach($mode in @('transfer','bfs','canonical')) {
            Measure-Run $mode $CandidateExe @("$N",'--algorithm',$mode) $run
        }
    }
    for($run=0;$run -le 3;$run++) {
        Measure-Run 'baseline-export9' $BaselineExe @("$N",'--export','--export-dir',"$OutputDir/baseline-export9-$run") $run
        Measure-Run 'canonical-export9' $CandidateExe @("$N",'--export','--compression-level','9','--export-dir',"$OutputDir/canonical-export9-$run") $run
        Measure-Run 'canonical-export1' $CandidateExe @("$N",'--export','--export-dir',"$OutputDir/canonical-export1-$run") $run
        Measure-Run 'canonical-raw' $CandidateExe @("$N",'--export','--no-compress','--export-dir',"$OutputDir/canonical-raw-$run") $run
    }
    $rows | ConvertTo-Json | Set-Content -LiteralPath "$OutputDir/results.json"
    $summary=foreach($group in ($rows | Group-Object name)) {
        $compute=@($group.Group.compute_seconds | Sort-Object)
        $process=@($group.Group.process_seconds | Sort-Object)
        [pscustomobject]@{name=$group.Name;runs=$group.Count;
            compute_median=$compute[[int][Math]::Floor($compute.Count/2)];
            process_median=$process[[int][Math]::Floor($process.Count/2)];
            compute_min=$compute[0];compute_max=$compute[-1]}
    }
    $summary | ConvertTo-Json | Set-Content -LiteralPath "$OutputDir/summary.json"
    $summary | Format-Table
    Get-FileHash -LiteralPath $BaselineExe,$CandidateExe | ConvertTo-Json | Set-Content -LiteralPath "$OutputDir/binary-hashes.json"
    Write-Output "Evidence: $OutputDir"
} finally {
    $env:RAYON_NUM_THREADS=$previousThreads
}
