<#
.SYNOPSIS
    Compare lua_cpp and lua_rust VM JSONL traces with fail-closed evidence handling.
.DESCRIPTION
    Executes every selected Lua source with --trace-diff, captures each process with a
    timeout, validates both JSONL traces, canonicalizes runtime identity addresses, and
    writes structured event differences. Missing binaries, missing/empty traces, invalid
    JSON, timeouts, unsupported trace options, and non-zero exits are failures.

    Representative mode is intended for pull-request checks. Full mode recursively
    executes every Lua source and is intended for nightly checks.
.PARAMETER InfrastructureSelfTest
    Uses a deterministic process fixture by default to exercise timeout-safe capture,
    JSONL parsing, normalization, comparison, and artifact generation. It makes no
    cross-language parity claim. Cpp and Rust self-comparison modes are also available
    to audit a real tool's trace support.
.EXAMPLE
    pwsh -File tools/compare_vm_trace.ps1 -InputPath tests/lua/bytecode/test_bytecode.lua
.EXAMPLE
    pwsh -File tools/compare_vm_trace.ps1 -InputPath tests/lua -CorpusMode Full
.EXAMPLE
    pwsh -File tools/compare_vm_trace.ps1 -InputPath tests/lua/step01_basic.lua `
        -InfrastructureSelfTest
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [Alias("InputDir")]
    [string]$InputPath,

    [ValidateSet("Representative", "Full")]
    [string]$CorpusMode = "Representative",

    [ValidateRange(1, 100000)]
    [int]$RepresentativeCount = 12,

    [string]$RepresentativeManifest = "",
    [string]$CppAppExe = "",
    [string]$RustAppExe = "",
    [string]$OutputDir = "",
    [string]$ResultPath = "",

    [ValidateRange(1, 3600)]
    [int]$TimeoutSeconds = 30,

    [ValidateSet("Synthetic", "Cpp", "Rust")]
    [string]$SelfTestTool = "Synthetic",

    [switch]$InfrastructureSelfTest,
    [switch]$JsonOutput
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDirectory ".."))
. (Join-Path $scriptDirectory "parity_runner_common.ps1")

function Test-TraceProperty {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Object,
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    if ($Object -is [System.Collections.IDictionary]) {
        return $Object.Contains($Name)
    }
    return $null -ne $Object.PSObject.Properties[$Name]
}

function Get-TraceProperty {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Object,
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    if ($Object -is [System.Collections.IDictionary]) {
        return $Object[$Name]
    }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function ConvertTo-NormalizedTraceNode {
    param(
        [AllowNull()]
        [object]$Value,
        [Parameter(Mandatory = $true)]
        [hashtable]$IdentityMap,
        [string]$PropertyName = ""
    )

    if ($null -eq $Value) {
        return $null
    }
    if ($Value -is [string]) {
        $text = [string]$Value
        if ($text -match '^(table|function|userdata|thread|lightuserdata):0x[0-9a-fA-F]+$') {
            if (-not $IdentityMap.ContainsKey($text)) {
                $IdentityMap[$text] = "$($Matches[1]):#$($IdentityMap.Count + 1)"
            }
            return $IdentityMap[$text]
        }
        if ($PropertyName -in @("source", "funcName")) {
            return $text.Replace('\', '/')
        }
        return $text
    }
    if ($Value -is [System.Collections.IDictionary]) {
        $map = [ordered]@{}
        foreach ($key in @($Value.Keys | Sort-Object)) {
            $map[[string]$key] = ConvertTo-NormalizedTraceNode -Value $Value[$key] `
                -IdentityMap $IdentityMap -PropertyName ([string]$key)
        }
        return $map
    }
    if ($Value.GetType() -eq [System.Management.Automation.PSCustomObject]) {
        $map = [ordered]@{}
        foreach ($property in @($Value.PSObject.Properties | Sort-Object Name)) {
            $map[$property.Name] = ConvertTo-NormalizedTraceNode -Value $property.Value `
                -IdentityMap $IdentityMap -PropertyName $property.Name
        }
        return $map
    }
    if ($Value -is [System.Collections.IEnumerable] -and $Value -isnot [string]) {
        $items = @($Value | ForEach-Object {
            ConvertTo-NormalizedTraceNode -Value $_ -IdentityMap $IdentityMap -PropertyName $PropertyName
        })
        return ,$items
    }
    return $Value
}

function Get-TraceCoverage {
    param(
        [Parameter(Mandatory = $true)]
        [object[]]$Events
    )

    $instructions = @($Events | Where-Object { (Get-TraceProperty -Object $_ -Name "kind") -eq "instruction" })
    $errors = @($Events | Where-Object { (Get-TraceProperty -Object $_ -Name "kind") -eq "error" })
    $upvalues = @($Events | Where-Object {
        (Get-TraceProperty -Object $_ -Name "kind") -in @("upvalue-open", "upvalue-close")
    })
    $controlEvents = @($Events | Where-Object {
        (Get-TraceProperty -Object $_ -Name "kind") -in @("call", "return", "yield", "resume")
    })
    $sequenceValid = $true
    for ($index = 0; $index -lt $Events.Count; $index++) {
        if (-not (Test-TraceProperty -Object $Events[$index] -Name "seq") -or
            [int64](Get-TraceProperty -Object $Events[$index] -Name "seq") -ne $index) {
            $sequenceValid = $false
            break
        }
    }

    return [pscustomobject][ordered]@{
        eventSequence       = $sequenceValid
        pcOpcode            = $instructions.Count -gt 0 -and @($instructions | Where-Object {
            -not (Test-TraceProperty -Object $_ -Name "pc") -or
            -not (Test-TraceProperty -Object $_ -Name "op")
        }).Count -eq 0
        activeCallFrames    = @($Events | Where-Object {
            -not (Test-TraceProperty -Object $_ -Name "callDepth") -or
            -not (Test-TraceProperty -Object $_ -Name "funcName")
        }).Count -eq 0
        stackTop            = $instructions.Count -gt 0 -and @($instructions | Where-Object {
            -not (Test-TraceProperty -Object $_ -Name "stackTop")
        }).Count -eq 0
        changedRegisters    = $instructions.Count -gt 0 -and @($instructions | Where-Object {
            -not (Test-TraceProperty -Object $_ -Name "changedRegisters")
        }).Count -eq 0
        upvalueLifecycle    = @($upvalues | Where-Object {
            -not (Test-TraceProperty -Object $_ -Name "slot") -or
            -not (Test-TraceProperty -Object $_ -Name "name")
        }).Count -eq 0
        callReturnYieldResume = @($controlEvents | Where-Object {
            -not (Test-TraceProperty -Object $_ -Name "callDepth")
        }).Count -eq 0
        errorValueCategory  = @($errors | Where-Object {
            (-not (Test-TraceProperty -Object $_ -Name "errorValue") -and
                -not (Test-TraceProperty -Object $_ -Name "value")) -or
            (-not (Test-TraceProperty -Object $_ -Name "errorCategory") -and
                -not (Test-TraceProperty -Object $_ -Name "category"))
        }).Count -eq 0
        observedEventKinds  = @($Events | ForEach-Object {
            [string](Get-TraceProperty -Object $_ -Name "kind")
        } | Sort-Object -Unique)
    }
}

function Convert-TraceFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return [pscustomobject][ordered]@{
            success = $false
            error = "trace file was not created; --trace-diff may be unsupported"
            events = @()
            coverage = $null
        }
    }
    if ((Get-Item -LiteralPath $Path).Length -eq 0) {
        return [pscustomobject][ordered]@{
            success = $false
            error = "trace file is empty; the tool emitted no observable VM events"
            events = @()
            coverage = $null
        }
    }

    $identityMap = @{}
    $events = New-Object System.Collections.ArrayList
    $lineNumber = 0
    try {
        foreach ($line in [System.IO.File]::ReadAllLines($Path)) {
            $lineNumber++
            if ([string]::IsNullOrWhiteSpace($line)) {
                continue
            }
            try {
                $event = $line | ConvertFrom-Json -ErrorAction Stop
            }
            catch {
                throw "invalid JSONL at line $lineNumber`: $($_.Exception.Message)"
            }
            if (-not (Test-TraceProperty -Object $event -Name "kind")) {
                throw "trace event at line $lineNumber has no kind"
            }
            [void]$events.Add((ConvertTo-NormalizedTraceNode -Value $event -IdentityMap $identityMap))
        }
        if ($events.Count -eq 0) {
            throw "trace contains no JSON events"
        }
        $normalized = @($events)
        return [pscustomobject][ordered]@{
            success = $true
            error = $null
            events = $normalized
            coverage = Get-TraceCoverage -Events $normalized
        }
    }
    catch {
        return [pscustomobject][ordered]@{
            success = $false
            error = $_.Exception.Message
            events = @($events)
            coverage = $null
        }
    }
}

function Normalize-TraceHostOutput {
    param(
        [AllowEmptyString()]
        [string]$Text
    )

    $kept = @()
    foreach ($line in ($Text -split "\r?\n")) {
        if ($line -match '^\[TRACE\]\s+(Trace(?: diff)? enabled|Trace complete):') {
            continue
        }
        if ($line -ne "" -or $kept.Count -gt 0) {
            $kept += $line
        }
    }
    return ($kept -join "`n").TrimEnd()
}

function Add-TraceDifference {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [System.Collections.ArrayList]$List,
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [string]$Kind,
        [AllowNull()][object]$Left,
        [AllowNull()][object]$Right,
        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    [void]$List.Add([pscustomobject][ordered]@{
        path = $Path
        kind = $Kind
        left = $Left
        right = $Right
        message = $Message
    })
}

function Get-TraceInvocation {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet("Cpp", "Rust", "Synthetic")]
        [string]$Adapter,
        [Parameter(Mandatory = $true)]
        [string]$Executable,
        [Parameter(Mandatory = $true)]
        [string]$TracePath,
        [Parameter(Mandatory = $true)]
        [string]$LuaInput,
        [Parameter(Mandatory = $true)]
        [string]$FixtureScript
    )

    if ($Adapter -eq "Synthetic") {
        return [pscustomobject][ordered]@{
            executable = $Executable
            arguments = @(
                "-NoProfile", "-NonInteractive", "-File", $FixtureScript,
                "-TracePath", $TracePath, "-InputPath", $LuaInput
            )
        }
    }
    return [pscustomobject][ordered]@{
        executable = $Executable
        arguments = @("--trace-diff", $TracePath, $LuaInput)
    }
}

$runId = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssfffZ")
$fixtureScript = Join-Path $scriptDirectory "parity_trace_fixture.ps1"
if (-not $CppAppExe) {
    $CppAppExe = Join-Path $projectRoot "..\lua_cpp\bin\lua_app.exe"
}
if (-not $RustAppExe) {
    $configuredTarget = Join-Path $projectRoot "target\x86_64-pc-windows-msvc\debug\lua_app.exe"
    $hostTarget = Join-Path $projectRoot "target\debug\lua_app.exe"
    $RustAppExe = if (Test-Path -LiteralPath $configuredTarget -PathType Leaf) {
        $configuredTarget
    }
    else {
        $hostTarget
    }
}
if (-not $OutputDir) {
    $OutputDir = Join-Path $projectRoot "target\parity\vm-trace"
}

$InputPath = Resolve-ParityPath -Path $InputPath -BasePath $projectRoot
$CppAppExe = Resolve-ParityPath -Path $CppAppExe -BasePath $projectRoot
$RustAppExe = Resolve-ParityPath -Path $RustAppExe -BasePath $projectRoot
$OutputDir = Resolve-ParityPath -Path $OutputDir -BasePath $projectRoot
if ($RepresentativeManifest) {
    $RepresentativeManifest = Resolve-ParityPath -Path $RepresentativeManifest -BasePath $projectRoot
}
if (-not $ResultPath) {
    $ResultPath = Join-Path $OutputDir "report.json"
}
else {
    $ResultPath = Resolve-ParityPath -Path $ResultPath -BasePath $projectRoot
}
$runDirectory = Join-Path (Join-Path $OutputDir "runs") $runId

$requiredEvidence = @(
    "eventSequence", "pcOpcode", "activeCallFrames", "stackTop",
    "changedRegisters", "upvalueLifecycle", "callReturnYieldResume",
    "errorValueCategory"
)
$caseResults = New-Object System.Collections.ArrayList
$preflightIssues = New-Object System.Collections.ArrayList
$report = [ordered]@{
    schemaVersion = 2
    runner = "compare_vm_trace"
    purpose = if ($InfrastructureSelfTest) { "infrastructure-self-test" } else { "cross-language-parity" }
    status = "running"
    generatedAt = [DateTime]::UtcNow.ToString("o")
    projectRoot = $projectRoot
    runDirectory = $runDirectory
    resultPath = $ResultPath
    corpus = [ordered]@{
        input = $InputPath
        mode = $CorpusMode
        representativeCount = $RepresentativeCount
        representativeManifest = if ($RepresentativeManifest) { $RepresentativeManifest } else { $null }
        selected = @()
    }
    tools = [ordered]@{
        cpp = $CppAppExe
        rust = $RustAppExe
    }
    requiredEvidence = $requiredEvidence
    preflightIssues = $preflightIssues
    summary = [ordered]@{
        selected = 0
        passed = 0
        failed = 0
        infrastructureFailures = 0
        semanticFailures = 0
    }
    observedEventKinds = @()
    cases = $caseResults
}

try {
    New-ParityDirectory -Path $runDirectory
}
catch {
    [Console]::Error.WriteLine("Cannot create trace parity output directory '$runDirectory': $($_.Exception.Message)")
    exit 2
}

$leftExecutable = $CppAppExe
$rightExecutable = $RustAppExe
$leftAdapter = "Cpp"
$rightAdapter = "Rust"
if ($InfrastructureSelfTest) {
    switch ($SelfTestTool) {
        "Synthetic" {
            $hostProcess = Get-Process -Id $PID
            $leftExecutable = $hostProcess.Path
            $rightExecutable = $hostProcess.Path
            $leftAdapter = "Synthetic"
            $rightAdapter = "Synthetic"
        }
        "Cpp" {
            $leftExecutable = $CppAppExe
            $rightExecutable = $CppAppExe
            $leftAdapter = "Cpp"
            $rightAdapter = "Cpp"
        }
        "Rust" {
            $leftExecutable = $RustAppExe
            $rightExecutable = $RustAppExe
            $leftAdapter = "Rust"
            $rightAdapter = "Rust"
        }
    }
}

if (-not (Test-Path -LiteralPath $InputPath)) {
    [void]$preflightIssues.Add("input path not found: $InputPath")
}
if (-not (Test-Path -LiteralPath $leftExecutable -PathType Leaf)) {
    [void]$preflightIssues.Add("left VM tool not found: $leftExecutable")
}
if (-not (Test-Path -LiteralPath $rightExecutable -PathType Leaf)) {
    [void]$preflightIssues.Add("right VM tool not found: $rightExecutable")
}
if (($leftAdapter -eq "Synthetic" -or $rightAdapter -eq "Synthetic") -and
    -not (Test-Path -LiteralPath $fixtureScript -PathType Leaf)) {
    [void]$preflightIssues.Add("trace self-test fixture not found: $fixtureScript")
}

if ($preflightIssues.Count -gt 0) {
    $report.status = "infrastructure-failed"
    $report.summary.infrastructureFailures = $preflightIssues.Count
    Write-ParityJson -Path $ResultPath -Value $report
    if ($JsonOutput) {
        $report | ConvertTo-Json -Depth 64 -Compress | Write-Output
    }
    [Console]::Error.WriteLine(("VM trace parity preflight failed. Report: {0}" -f $ResultPath))
    exit 2
}

try {
    $files = @(Select-ParityCorpus -InputPath $InputPath -Mode $CorpusMode `
        -RepresentativeCount $RepresentativeCount -RepresentativeManifest $RepresentativeManifest)
}
catch {
    [void]$preflightIssues.Add($_.Exception.Message)
    $report.status = "infrastructure-failed"
    $report.summary.infrastructureFailures = 1
    Write-ParityJson -Path $ResultPath -Value $report
    if ($JsonOutput) {
        $report | ConvertTo-Json -Depth 64 -Compress | Write-Output
    }
    [Console]::Error.WriteLine(("VM trace corpus selection failed. Report: {0}" -f $ResultPath))
    exit 2
}

$corpusRoot = if ((Get-Item -LiteralPath $InputPath).PSIsContainer) {
    $InputPath
}
else {
    Split-Path -Parent $InputPath
}
$report.corpus.selected = @($files | ForEach-Object {
    Get-ParityRelativePath -Root $corpusRoot -Path $_.FullName
})
$report.summary.selected = $files.Count
$allObservedKinds = New-Object System.Collections.ArrayList

foreach ($file in $files) {
    $relativePath = Get-ParityRelativePath -Root $corpusRoot -Path $file.FullName
    $caseId = Get-ParityCaseId -RelativePath $relativePath
    $caseDirectory = Join-Path $runDirectory $caseId
    New-ParityDirectory -Path $caseDirectory
    $leftTracePath = Join-Path $caseDirectory "left.trace.jsonl"
    $rightTracePath = Join-Path $caseDirectory "right.trace.jsonl"
    $toolInput = $file.FullName.Replace('\', '/')

    $leftInvocation = Get-TraceInvocation -Adapter $leftAdapter -Executable $leftExecutable `
        -TracePath $leftTracePath -LuaInput $toolInput -FixtureScript $fixtureScript
    $rightInvocation = Get-TraceInvocation -Adapter $rightAdapter -Executable $rightExecutable `
        -TracePath $rightTracePath -LuaInput $toolInput -FixtureScript $fixtureScript
    $leftRaw = Invoke-ParityProcess -Executable $leftInvocation.executable -Arguments $leftInvocation.arguments `
        -WorkingDirectory $projectRoot -TimeoutSeconds $TimeoutSeconds
    $rightRaw = Invoke-ParityProcess -Executable $rightInvocation.executable -Arguments $rightInvocation.arguments `
        -WorkingDirectory $projectRoot -TimeoutSeconds $TimeoutSeconds
    $leftExecution = Save-ParityExecution -Execution $leftRaw -CaseDirectory $caseDirectory -Side "left"
    $rightExecution = Save-ParityExecution -Execution $rightRaw -CaseDirectory $caseDirectory -Side "right"

    $differences = New-Object System.Collections.ArrayList
    $infrastructureFailure = $false
    foreach ($side in @(
        [pscustomobject]@{ name = "left"; execution = $leftExecution },
        [pscustomobject]@{ name = "right"; execution = $rightExecution }
    )) {
        if ($side.execution.startError) {
            $infrastructureFailure = $true
            Add-TraceDifference -List $differences -Path "$($side.name).process" -Kind "start-error" `
                -Left $side.execution.startError -Right $null -Message "VM tool could not be started"
        }
        if ($side.execution.timedOut) {
            $infrastructureFailure = $true
            Add-TraceDifference -List $differences -Path "$($side.name).process" -Kind "timeout" `
                -Left $TimeoutSeconds -Right $null -Message "VM tool exceeded timeout"
        }
        if ($null -ne $side.execution.exitCode -and $side.execution.exitCode -ne 0) {
            $infrastructureFailure = $true
            Add-TraceDifference -List $differences -Path "$($side.name).exitCode" -Kind "process-exit" `
                -Left $side.execution.exitCode -Right 0 -Message "VM tool exited unsuccessfully"
        }
    }

    $leftParsed = if (-not $leftExecution.startError -and -not $leftExecution.timedOut) {
        Convert-TraceFile -Path $leftTracePath
    }
    else {
        [pscustomobject]@{ success = $false; error = "process did not complete"; events = @(); coverage = $null }
    }
    $rightParsed = if (-not $rightExecution.startError -and -not $rightExecution.timedOut) {
        Convert-TraceFile -Path $rightTracePath
    }
    else {
        [pscustomobject]@{ success = $false; error = "process did not complete"; events = @(); coverage = $null }
    }
    if (-not $leftParsed.success) {
        $infrastructureFailure = $true
        Add-TraceDifference -List $differences -Path "left.trace" -Kind "trace-unavailable" `
            -Left $leftParsed.error -Right $null -Message "left tool produced no valid VM trace"
    }
    if (-not $rightParsed.success) {
        $infrastructureFailure = $true
        Add-TraceDifference -List $differences -Path "right.trace" -Kind "trace-unavailable" `
            -Left $rightParsed.error -Right $null -Message "right tool produced no valid VM trace"
    }

    if ($leftParsed.success -and $rightParsed.success) {
        foreach ($kind in @($leftParsed.coverage.observedEventKinds + $rightParsed.coverage.observedEventKinds)) {
            if (-not ($allObservedKinds -contains $kind)) {
                [void]$allObservedKinds.Add($kind)
            }
        }
        if (-not $InfrastructureSelfTest) {
            foreach ($evidence in $requiredEvidence) {
                $leftCovered = [bool]$leftParsed.coverage.$evidence
                $rightCovered = [bool]$rightParsed.coverage.$evidence
                if (-not ($leftCovered -and $rightCovered)) {
                    Add-TraceDifference -List $differences -Path "evidence.$evidence" `
                        -Kind "missing-evidence" -Left $leftCovered -Right $rightCovered `
                        -Message "required VM trace evidence is unavailable on one or both sides"
                }
            }
        }

        $eventComparison = Compare-ParityValue -Left $leftParsed.events -Right $rightParsed.events `
            -Path '$.events' -MaximumDifferences 500
        foreach ($difference in $eventComparison.items) {
            [void]$differences.Add($difference)
        }
        if ($eventComparison.truncated) {
            Add-TraceDifference -List $differences -Path '$.events' -Kind "truncated" `
                -Left 500 -Right $null -Message "event difference list reached its configured limit"
        }

        $leftStdout = Normalize-TraceHostOutput -Text $leftExecution.stdout
        $rightStdout = Normalize-TraceHostOutput -Text $rightExecution.stdout
        if ($leftStdout -cne $rightStdout) {
            Add-TraceDifference -List $differences -Path '$.process.stdout' -Kind "value" `
                -Left $leftStdout -Right $rightStdout -Message "observable stdout differs after trace diagnostics are removed"
        }
        if ($leftExecution.stderr -cne $rightExecution.stderr) {
            Add-TraceDifference -List $differences -Path '$.process.stderr' -Kind "value" `
                -Left $leftExecution.stderr -Right $rightExecution.stderr -Message "observable stderr differs"
        }
    }

    Write-ParityJson -Path (Join-Path $caseDirectory "left.normalized.json") -Value $leftParsed
    Write-ParityJson -Path (Join-Path $caseDirectory "right.normalized.json") -Value $rightParsed
    $caseStatus = if ($differences.Count -eq 0) { "passed" } else { "failed" }
    $sourceCopy = $null
    if ($caseStatus -eq "failed") {
        $sourceCopy = Join-Path $caseDirectory "input.lua"
        Copy-Item -LiteralPath $file.FullName -Destination $sourceCopy -Force
    }

    $caseResult = [pscustomobject][ordered]@{
        id = $caseId
        input = $file.FullName
        relativeInput = $relativePath
        inputSha256 = Get-ParityFileSha256 -Path $file.FullName
        sourceCopy = $sourceCopy
        status = $caseStatus
        infrastructureFailure = $infrastructureFailure
        evidence = [ordered]@{
            left = $leftParsed.coverage
            right = $rightParsed.coverage
        }
        executions = [ordered]@{
            left = $leftExecution
            right = $rightExecution
        }
        traces = [ordered]@{
            left = $leftTracePath
            right = $rightTracePath
        }
        differences = @($differences)
        differenceCount = $differences.Count
        artifact = Join-Path $caseDirectory "case.json"
    }
    Write-ParityJson -Path $caseResult.artifact -Value $caseResult
    [void]$caseResults.Add($caseResult)

    if ($caseStatus -eq "passed") {
        $report.summary.passed++
        Write-Host "[PASS] $relativePath"
    }
    else {
        $report.summary.failed++
        if ($infrastructureFailure) {
            $report.summary.infrastructureFailures++
        }
        else {
            $report.summary.semanticFailures++
        }
        Write-Host "[FAIL] $relativePath -> $($caseResult.artifact)" -ForegroundColor Red
    }
}

$report.observedEventKinds = @($allObservedKinds | Sort-Object)
if ($report.summary.failed -eq 0) {
    $report.status = if ($InfrastructureSelfTest) { "self-test-passed" } else { "passed" }
}
elseif ($report.summary.infrastructureFailures -gt 0) {
    $report.status = "infrastructure-failed"
}
else {
    $report.status = "differences-found"
}
$report.completedAt = [DateTime]::UtcNow.ToString("o")
$report.selfTestNotice = if ($InfrastructureSelfTest) {
    "This result proves runner behavior only; it is not a lua_cpp/lua_rust VM parity result."
}
else {
    $null
}
Write-ParityJson -Path $ResultPath -Value $report

Write-Host ""
Write-Host "VM trace parity: $($report.status)"
Write-Host "  selected=$($report.summary.selected) passed=$($report.summary.passed) failed=$($report.summary.failed)"
Write-Host "  report=$ResultPath"
if ($JsonOutput) {
    $report | ConvertTo-Json -Depth 64 -Compress | Write-Output
}

if ($report.summary.infrastructureFailures -gt 0) {
    exit 2
}
if ($report.summary.failed -gt 0) {
    exit 1
}
exit 0
