[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$CppBytecodeExe,

    [Parameter(Mandatory = $true)]
    [string]$RustBytecodeExe,

    [string]$OutputDir = "target/parity/bytecode-tool-contract",

    [ValidateRange(1, 3600)]
    [int]$TimeoutSeconds = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDirectory ".."))
. (Join-Path $scriptDirectory "parity_runner_common.ps1")

$CppBytecodeExe = Resolve-ParityPath -Path $CppBytecodeExe -BasePath $projectRoot
$RustBytecodeExe = Resolve-ParityPath -Path $RustBytecodeExe -BasePath $projectRoot
$OutputDir = Resolve-ParityPath -Path $OutputDir -BasePath $projectRoot
$resultPath = Join-Path $OutputDir "report.json"
$fixtureRoot = Join-Path $OutputDir "fixtures"
New-ParityDirectory -Path $OutputDir
New-ParityDirectory -Path $fixtureRoot

$utf8 = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText(
    (Join-Path $fixtureRoot "simple.lua"),
    'return "quote\" slash\\ line\n tab\t", 42',
    $utf8
)
[System.IO.File]::WriteAllText(
    (Join-Path $fixtureRoot "nested.lua"),
    "local function child(x)`n  if x then return `"yes`" else return `"no`" end`nend`nreturn child`n",
    $utf8
)
[System.IO.File]::WriteAllText(
    (Join-Path $fixtureRoot "different.lua"),
    "return 43`n",
    $utf8
)
[System.IO.File]::WriteAllText(
    (Join-Path $fixtureRoot "invalid.lua"),
    "local =`n",
    $utf8
)

$usage = "Usage:`n  bytecode_main <script.lua> [full|--full]`n  bytecode_main <script.lua> --cfg [full|--full]`n  bytecode_main <left.lua> <right.lua> --diff [full|--full]`n"
$crossLanguageCases = @(
    [ordered]@{ id = "no-arguments"; arguments = @(); exit = 1; stdout = @(); stderrExact = $usage },
    [ordered]@{ id = "unknown-option"; arguments = @("--bogus"); exit = 1; stdout = @(); stderrExact = "[ERROR] Unknown option: --bogus`n$usage" },
    [ordered]@{ id = "wrong-print-count"; arguments = @("simple.lua", "nested.lua"); exit = 1; stdout = @(); stderr = @("Expected 1 script path for print mode, got 2") },
    [ordered]@{ id = "cfg-diff-conflict"; arguments = @("simple.lua", "nested.lua", "--cfg", "--diff"); exit = 1; stdout = @(); stderr = @("--cfg cannot be combined with --diff") },
    [ordered]@{ id = "compact-text"; arguments = @("simple.lua"); exit = 0; stdout = @("Proto", "K[0] = string `"quote\`" slash\\ line\n tab\t`"", "K[1] = number 42"); stderr = @() },
    [ordered]@{ id = "full-nested"; arguments = @("nested.lua", "--full"); exit = 0; stdout = @("child protos (1)", "proto[0] nested.lua:1", "source: nested.lua"); stderr = @() },
    [ordered]@{ id = "cfg-compact"; arguments = @("nested.lua", "--cfg"); exit = 0; stdout = @("flowchart TD", "%% lua_bytecode Mermaid CFG", "subgraph P0"); stderr = @() },
    [ordered]@{ id = "cfg-full"; arguments = @("nested.lua", "--cfg", "full"); exit = 0; stdout = @("subgraph P0", "subgraph P1", "proto[0] nested.lua:1"); stderr = @() },
    [ordered]@{ id = "diff-identical"; arguments = @("simple.lua", "simple.lua", "--diff"); exit = 0; stdout = @("Bytecode diff", "status: identical", "changed lines: 0"); stderr = @() },
    [ordered]@{ id = "diff-different"; arguments = @("simple.lua", "different.lua", "--diff"); exit = 0; stdout = @("Bytecode diff", "status: different", "line | left | right"); stderr = @() },
    [ordered]@{ id = "missing-file"; arguments = @("missing.lua"); exit = 3; stdout = @(); stderrExact = "[ERROR] Exception: cannot open file: missing.lua`n" },
    [ordered]@{ id = "syntax-error"; arguments = @("invalid.lua"); exit = 3; stdout = @(); stderr = @("[ERROR] Exception:", "Expected variable name in local statement") }
)

$rustOnlyCases = @(
    [ordered]@{ id = "json-schema"; arguments = @("nested.lua", "--format=json"); exit = 0; json = $true },
    [ordered]@{ id = "unknown-format"; arguments = @("simple.lua", "--format=yaml"); exit = 1; stderr = @("Unknown format: yaml (expected text or json)") },
    [ordered]@{ id = "json-mode-conflict"; arguments = @("nested.lua", "--format=json", "--full"); exit = 1; stderr = @("--format=json cannot be combined with --cfg, --diff, or --full") }
)

$issues = New-Object System.Collections.ArrayList
$caseReports = @()

function Test-ContractExecution {
    param(
        [Parameter(Mandatory = $true)] [object]$Case,
        [Parameter(Mandatory = $true)] [string]$Side,
        [Parameter(Mandatory = $true)] [object]$Execution,
        [Parameter(Mandatory = $true)] [AllowEmptyCollection()] [System.Collections.ArrayList]$CaseIssues
    )

    $stdout = $Execution.stdout.Replace("`r`n", "`n")
    $stderr = $Execution.stderr.Replace("`r`n", "`n")
    if ($Execution.timedOut) { [void]$CaseIssues.Add("$Side timed out") }
    if ($null -ne $Execution.startError) { [void]$CaseIssues.Add("$Side start error: $($Execution.startError)") }
    if ($Execution.exitCode -ne $Case.exit) { [void]$CaseIssues.Add("$Side exit $($Execution.exitCode), expected $($Case.exit)") }
    if ($Case.Contains("stderrExact") -and $stderr -cne $Case.stderrExact) {
        [void]$CaseIssues.Add("$Side stderr did not match the fixed bytecode tool contract")
    }
    if ($Case.Contains("stdout")) {
        foreach ($fragment in @($Case.stdout)) {
            if (-not $stdout.Contains($fragment)) { [void]$CaseIssues.Add("$Side stdout lacks '$fragment'") }
        }
    }
    if ($Case.Contains("stderr")) {
        foreach ($fragment in @($Case.stderr)) {
            if (-not $stderr.Contains($fragment)) { [void]$CaseIssues.Add("$Side stderr lacks '$fragment'") }
        }
    }
    if ($Case.exit -ne 0 -and $stdout.Length -ne 0) {
        [void]$CaseIssues.Add("$Side emitted partial stdout on failure")
    }
    if ($Case.Contains("json") -and $Case.json) {
        try {
            $document = $stdout | ConvertFrom-Json
            if ($document.schema_version -ne 2 -or $document.input -ne "nested.lua" -or
                $document.child_count -ne 1 -or $document.sub_protos.Count -ne 1) {
                [void]$CaseIssues.Add("$Side JSON lacks the stable recursive schema-2 contract")
            }
        }
        catch {
            [void]$CaseIssues.Add("$Side JSON is invalid: $($_.Exception.Message)")
        }
    }
}

foreach ($case in $crossLanguageCases) {
    $caseDirectory = Join-Path $OutputDir $case.id
    New-ParityDirectory -Path $caseDirectory
    $sideReports = [ordered]@{}
    foreach ($side in @(
        [ordered]@{ name = "cpp"; executable = $CppBytecodeExe },
        [ordered]@{ name = "rust"; executable = $RustBytecodeExe }
    )) {
        $execution = Invoke-ParityProcess -Executable $side.executable -Arguments $case.arguments `
            -WorkingDirectory $fixtureRoot -TimeoutSeconds $TimeoutSeconds
        $saved = Save-ParityExecution -Execution $execution -CaseDirectory $caseDirectory -Side $side.name
        $sideReports[$side.name] = $saved
    }
    $caseIssues = New-Object System.Collections.ArrayList
    Test-ContractExecution -Case $case -Side "cpp" -Execution $sideReports.cpp -CaseIssues $caseIssues
    Test-ContractExecution -Case $case -Side "rust" -Execution $sideReports.rust -CaseIssues $caseIssues
    if ($sideReports.cpp.stdout -cne $sideReports.rust.stdout -or
        $sideReports.cpp.stderr -cne $sideReports.rust.stderr -or
        $sideReports.cpp.exitCode -ne $sideReports.rust.exitCode) {
        [void]$caseIssues.Add("cross-language bytecode tool evidence differs")
    }
    foreach ($issue in $caseIssues) { [void]$issues.Add("$($case.id): $issue") }
    $caseReports += [ordered]@{
        id = $case.id
        scope = "cross-language"
        status = if ($caseIssues.Count -eq 0) { "passed" } else { "failed" }
        executions = $sideReports
        issues = @($caseIssues)
    }
}

foreach ($case in $rustOnlyCases) {
    $caseDirectory = Join-Path $OutputDir $case.id
    New-ParityDirectory -Path $caseDirectory
    $execution = Invoke-ParityProcess -Executable $RustBytecodeExe -Arguments $case.arguments `
        -WorkingDirectory $fixtureRoot -TimeoutSeconds $TimeoutSeconds
    $saved = Save-ParityExecution -Execution $execution -CaseDirectory $caseDirectory -Side "rust"
    $caseIssues = New-Object System.Collections.ArrayList
    Test-ContractExecution -Case $case -Side "rust" -Execution $saved -CaseIssues $caseIssues
    foreach ($issue in $caseIssues) { [void]$issues.Add("$($case.id): $issue") }
    $caseReports += [ordered]@{
        id = $case.id
        scope = "rust-extension"
        status = if ($caseIssues.Count -eq 0) { "passed" } else { "failed" }
        executions = [ordered]@{ rust = $saved }
        issues = @($caseIssues)
    }
}

$passed = @($caseReports | Where-Object { $_.status -eq "passed" }).Count
$report = [ordered]@{
    schemaVersion = 1
    runner = "test_bytecode_tool_contract"
    purpose = "bytecode-viewer-contract"
    status = if ($issues.Count -eq 0) { "passed" } else { "failed" }
    summary = [ordered]@{
        selected = $caseReports.Count
        passed = $passed
        failed = $caseReports.Count - $passed
        crossLanguage = $crossLanguageCases.Count
        rustExtensions = $rustOnlyCases.Count
    }
    covered = @("arguments", "exit-status", "compact", "full", "cfg", "diff", "string-escaping", "nested-proto", "file-error", "compile-error", "json-schema", "strict-format")
    cases = @($caseReports)
    issues = @($issues)
}
Write-ParityJson -Path $resultPath -Value $report

if ($issues.Count -ne 0) {
    foreach ($issue in $issues) { [Console]::Error.WriteLine("[FAIL] $issue") }
    [Console]::Error.WriteLine("Bytecode tool contract failed: $resultPath")
    exit 1
}

Write-Output "[PASS] Bytecode tool contract: $passed/$($caseReports.Count) cases"
Write-Output "  report=$resultPath"
exit 0
