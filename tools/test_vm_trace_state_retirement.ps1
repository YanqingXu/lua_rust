[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("Cpp", "Rust")]
    [string]$Tool,

    [Parameter(Mandatory = $true)]
    [string]$Executable,

    [string]$InputPath = "tests/trace_fixtures/coroutine_state_retirement.lua",
    [string]$OutputDir = "target/parity/vm-trace-state-retirement",

    [ValidateRange(1, 3600)]
    [int]$TimeoutSeconds = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDirectory ".."))
. (Join-Path $scriptDirectory "parity_runner_common.ps1")

$Executable = Resolve-ParityPath -Path $Executable -BasePath $projectRoot
$InputPath = Resolve-ParityPath -Path $InputPath -BasePath $projectRoot
$OutputDir = Resolve-ParityPath -Path $OutputDir -BasePath $projectRoot
$tracePath = Join-Path $OutputDir "trace.jsonl"
$resultPath = Join-Path $OutputDir "report.json"
New-ParityDirectory -Path $OutputDir

$execution = Invoke-ParityProcess -Executable $Executable `
    -Arguments @("--trace-diff", $tracePath, $InputPath.Replace('\', '/')) `
    -WorkingDirectory $projectRoot -TimeoutSeconds $TimeoutSeconds
$savedExecution = Save-ParityExecution -Execution $execution -CaseDirectory $OutputDir -Side "process"
$issues = New-Object System.Collections.ArrayList
$events = @()

if ($savedExecution.startError) {
    [void]$issues.Add("trace producer could not be started: $($savedExecution.startError)")
}
if ($savedExecution.timedOut) {
    [void]$issues.Add("trace producer exceeded timeout")
}
if ($savedExecution.exitCode -ne 1) {
    [void]$issues.Add("fixture must terminate with the expected unhandled error (exit 1)")
}
if (-not (Test-Path -LiteralPath $tracePath -PathType Leaf)) {
    [void]$issues.Add("trace producer did not create a trace file")
}
else {
    $lineNumber = 0
    foreach ($line in Get-Content -LiteralPath $tracePath) {
        ++$lineNumber
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        try {
            $events += $line | ConvertFrom-Json
        }
        catch {
            [void]$issues.Add("trace line $lineNumber is not valid JSON")
            break
        }
    }
}
if ($events.Count -eq 0) {
    [void]$issues.Add("trace contains no events")
}

$openEvents = @($events | Where-Object { $_.kind -eq "upvalue-open" -and $_.name -eq "captured" })
$closeEvents = @($events | Where-Object { $_.kind -eq "upvalue-close" -and $_.name -eq "captured" })
$allOpenEvents = @($events | Where-Object { $_.kind -eq "upvalue-open" })
$allCloseEvents = @($events | Where-Object { $_.kind -eq "upvalue-close" })
$observedKinds = @($events.kind | Sort-Object -Unique)
$expectedKinds = @("call", "error", "instruction", "resume", "return", "upvalue-close", "upvalue-open", "yield")

if ($openEvents.Count -ne 3) {
    [void]$issues.Add("expected exactly 3 captured upvalue-open events, observed $($openEvents.Count)")
}
if ($closeEvents.Count -ne 2) {
    [void]$issues.Add("expected exactly 2 GC-retirement captured upvalue-close events, observed $($closeEvents.Count)")
}
if ($allOpenEvents.Count -ne 6 -or $allCloseEvents.Count -ne 5) {
    [void]$issues.Add("expected 6 total opens and 5 total closes, observed $($allOpenEvents.Count)/$($allCloseEvents.Count)")
}
foreach ($kind in $expectedKinds) {
    if (-not ($observedKinds -contains $kind)) {
        [void]$issues.Add("trace did not observe required event kind '$kind'")
    }
}
for ($index = 0; $index -lt [System.Math]::Min($openEvents.Count, $closeEvents.Count); ++$index) {
    if ([int64]$closeEvents[$index].seq -le [int64]$openEvents[$index].seq -or
        [int64]$closeEvents[$index].seq -ge [int64]$openEvents[$index + 1].seq) {
        [void]$issues.Add("captured close $index was not emitted between its open and the next state incarnation")
    }
}

$report = [ordered]@{
    schemaVersion = 1
    runner = "test_vm_trace_state_retirement"
    purpose = "producer-lifecycle-self-test"
    status = if ($issues.Count -eq 0) { "passed" } else { "failed" }
    tool = $Tool
    input = $InputPath
    traceMode = "Diff"
    evidenceProfile = "Complete"
    execution = $savedExecution
    observed = [ordered]@{
        eventCount = $events.Count
        eventKinds = $observedKinds
        totalUpvalueOpens = $allOpenEvents.Count
        totalUpvalueCloses = $allCloseEvents.Count
        capturedUpvalueOpens = $openEvents.Count
        capturedUpvalueCloses = $closeEvents.Count
    }
    issues = @($issues)
    trace = $tracePath
}
Write-ParityJson -Path $resultPath -Value $report

if ($issues.Count -ne 0) {
    foreach ($issue in $issues) {
        [Console]::Error.WriteLine("[FAIL] $issue")
    }
    [Console]::Error.WriteLine("VM trace state-retirement self-test failed: $resultPath")
    exit 1
}

Write-Output "[PASS] $Tool VM trace state retirement: 3 captured opens / 2 GC closes"
Write-Output "  report=$resultPath"
exit 0
