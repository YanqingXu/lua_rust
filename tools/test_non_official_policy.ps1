param([string]$ResultPath = "target/compatibility/non-official-policy-self-test.json")
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot "non_official_policy.ps1")

function New-PolicyTestContext {
    $fixtures = @()
    $cases = @()
    foreach ($id in @("debt.case", "passing.case")) {
        $fixtures += [ordered]@{
            id = $id; path = "tests/lua/$id.lua"; kind = "entry"; expected_exit = 0
            compare_stdout = $true; compare_stderr = $true; compare_side_effects = $true
        }
        $isDebt = $id -eq "debt.case"
        $executions = @{}
        foreach ($engine in @("lua_rust", "lua_cpp")) {
            $output = if ($isDebt -and $engine -eq "lua_rust") { "Memory: 10`n" } else { "Memory: 20`n" }
            $executions[$engine] = [ordered]@{
                timed_out = $false; spawn_error = $null; exit_code = 0; expected_exit_matched = $true
                stdout = @{ normalized_lossy_text = $output }
                stderr = @{ lossy_text = ""; normalized_lossy_text = "" }
            }
        }
        $cases += [ordered]@{
            id = $id; path = "tests/lua/$id.lua"; kind = "entry"; expected_exit = 0
            status = if ($isDebt) { "difference" } else { "match" }
            lua_rust = $executions.lua_rust; lua_cpp = $executions.lua_cpp
            comparison = [ordered]@{
                timeout_equal = $true; exit_code_equal = $true
                stdout_compared = $true; stdout_equal = -not $isDebt
                stderr_compared = $true; stderr_equal = $true
                side_effects_compared = $true; side_effects_equal = $true
                matched = -not $isDebt
            }
        }
    }
    $context = [ordered]@{
        Manifest = @{ inventory = @{ current_total = 2 }; cases = $fixtures }
        Policy = @{
            schema_version = 1; suite = "non-official"; required_cases = $fixtures
            debts = @(@{
                id = "debt.case"; channels = @("stdout"); reason = "Memory accounting only"
                stdout_patterns = @{ lua_rust = '\AMemory: [0-9]+\n\z'; lua_cpp = '\AMemory: [0-9]+\n\z' }
            })
        }
        Artifact = @{
            schema_version = "lua-fixture-differential/v1"; suite = "non-official"
            fixture_inventory = @{ current_total = 2 }
            summary = @{ selected = 2; executed = 2; helpers_skipped = 0; matches = 1
                differences = 1; runner_errors = 0; timed_out = 0; rust_only_expected = 0 }
            cases = $cases
        }
    }
    # Match ConvertFrom-Json's representation used by the real gate.
    return $context | ConvertTo-Json -Depth 15 | ConvertFrom-Json
}

$checks = [System.Collections.Generic.List[object]]::new()
function Assert-Policy {
    param([string]$Name, [scriptblock]$Mutate, [bool]$ExpectedPass, [string]$Failure = "")
    $context = New-PolicyTestContext
    & $Mutate $context
    $result = Test-NonOfficialArtifact -Artifact $context.Artifact -Manifest $context.Manifest -Policy $context.Policy
    if ($result.passed -ne $ExpectedPass -or
        (-not $ExpectedPass -and ($result.failures -join "`n") -notlike "*$Failure*")) {
        throw "self-test '$Name' failed: $($result | ConvertTo-Json -Depth 8 -Compress)"
    }
    $checks.Add([ordered]@{ name = $Name; passed = $true; observedFailures = @($result.failures) })
}

Assert-Policy "bounded historical stdout remains debt" {} $true
Assert-Policy "same difference count cannot hide a different failing case" {
    param($c)
    $c.Artifact.cases[0].status = "match"
    $c.Artifact.cases[0].comparison.stdout_equal = $true
    $c.Artifact.cases[0].comparison.matched = $true
    $c.Artifact.cases[1].status = "difference"
    $c.Artifact.cases[1].comparison.stdout_equal = $false
    $c.Artifact.cases[1].comparison.matched = $false
} $false "unapproved semantic regression: passing.case"
Assert-Policy "new stdout behavior in a debt case fails" {
    param($c) $c.Artifact.cases[0].lua_rust.stdout.normalized_lossy_text = "wrong result`n"
} $false "changed debt stdout signature"
Assert-Policy "new stderr channel in a debt case fails" {
    param($c) $c.Artifact.cases[0].comparison.stderr_equal = $false
} $false "new difference channel"
Assert-Policy "a newly added failing fixture has no implicit waiver" {
    param($c)
    $newFixture = $c.Manifest.cases[1] | ConvertTo-Json | ConvertFrom-Json
    $newCase = $c.Artifact.cases[1] | ConvertTo-Json -Depth 8 | ConvertFrom-Json
    $newFixture.id = "new.case"
    $newCase.id = "new.case"
    $newCase.status = "difference"
    $newCase.comparison.stdout_equal = $false
    $newCase.comparison.matched = $false
    $c.Manifest.cases += $newFixture
    $c.Artifact.cases += $newCase
    $c.Manifest.inventory.current_total = 3
    $c.Artifact.fixture_inventory.current_total = 3
    $c.Artifact.summary.selected = 3
    $c.Artifact.summary.executed = 3
    $c.Artifact.summary.differences = 2
} $false "unapproved semantic regression: new.case"
Assert-Policy "disabled comparison cannot hide regression" {
    param($c) $c.Artifact.cases[0].comparison.stdout_compared = $false
} $false "comparison channel disabled or changed"
Assert-Policy "resolved debt requires removing its allowance" {
    param($c)
    $c.Artifact.cases[0].status = "match"
    $c.Artifact.cases[0].comparison.stdout_equal = $true
    $c.Artifact.cases[0].comparison.matched = $true
    $c.Artifact.summary.matches = 2
    $c.Artifact.summary.differences = 0
} $false "resolved debt must be removed"
Assert-Policy "resolved case passes once allowance is removed" {
    param($c)
    $c.Policy.debts = @()
    $c.Artifact.cases[0].status = "match"
    $c.Artifact.cases[0].comparison.stdout_equal = $true
    $c.Artifact.cases[0].comparison.matched = $true
    $c.Artifact.summary.matches = 2
    $c.Artifact.summary.differences = 0
} $true
Assert-Policy "matching erroneous exits cannot be waived" {
    param($c)
    $c.Artifact.cases[0].lua_rust.exit_code = 1
    $c.Artifact.cases[0].lua_cpp.exit_code = 1
    $c.Artifact.cases[0].lua_rust.expected_exit_matched = $false
    $c.Artifact.cases[0].lua_cpp.expected_exit_matched = $false
} $false "abnormal or unexpected process exit"
Assert-Policy "Windows process abort fails" {
    param($c) $c.Artifact.cases[0].lua_rust.exit_code = -1073740791
} $false "abnormal or unexpected process exit"
Assert-Policy "Unix signal termination fails" {
    param($c) $c.Artifact.cases[0].lua_rust.exit_code = $null
} $false "abnormal or unexpected process exit"
Assert-Policy "panic cannot be allowed by changing expected exit" {
    param($c)
    $c.Manifest.cases[0].expected_exit = 101
    $c.Artifact.cases[0].expected_exit = 101
    $c.Artifact.cases[0].lua_rust.exit_code = 101
    $c.Artifact.cases[0].lua_cpp.exit_code = 101
} $false "abnormal or unexpected process exit"
Assert-Policy "panic text fails even if exit is masked" {
    param($c) $c.Artifact.cases[0].lua_rust.stderr.lossy_text = "thread 'main' panicked at src/lib.rs:1"
} $false "host panic or sanitizer failure"
Assert-Policy "timeout fails" {
    param($c)
    $c.Artifact.cases[0].lua_rust.timed_out = $true
    $c.Artifact.summary.timed_out = 1
} $false "abnormal or unexpected process exit"
Assert-Policy "spawn failure fails" {
    param($c) $c.Artifact.cases[0].lua_rust.spawn_error = "executable unavailable"
} $false "abnormal or unexpected process exit"
Assert-Policy "missing artifact case fails with corrected totals" {
    param($c)
    $c.Artifact.cases = @($c.Artifact.cases[0])
    $c.Artifact.summary.selected = 1
    $c.Artifact.summary.executed = 1
    $c.Artifact.summary.matches = 0
} $false "manifest case missing from artifact"
Assert-Policy "deleting a baseline passing case from both inputs fails" {
    param($c)
    $c.Artifact.cases = @($c.Artifact.cases[0])
    $c.Manifest.cases = @($c.Manifest.cases[0])
    $c.Artifact.summary.selected = 1
    $c.Artifact.summary.executed = 1
    $c.Artifact.summary.matches = 0
} $false "required baseline case missing from manifest"
Assert-Policy "replacing passing case ID cannot evade baseline" {
    param($c)
    $c.Artifact.cases[1].id = "replacement.case"
    $c.Manifest.cases[1].id = "replacement.case"
} $false "required baseline case missing from manifest"
Assert-Policy "converting a passing case to a skipped helper fails" {
    param($c)
    $c.Manifest.cases[1].kind = "helper"
    $c.Artifact.cases[1].kind = "helper"
    $c.Artifact.cases[1].status = "helper-skipped"
    $c.Artifact.cases[1].lua_rust = $null
    $c.Artifact.cases[1].lua_cpp = $null
    $c.Artifact.cases[1].comparison = $null
    $c.Artifact.summary.helpers_skipped = 1
    $c.Artifact.summary.executed = 1
    $c.Artifact.summary.matches = 0
} $false "baseline case execution contract changed"
Assert-Policy "duplicate artifact case fails" {
    param($c) $c.Artifact.cases += $c.Artifact.cases[0]
} $false "duplicate artifact case"
Assert-Policy "forged summary does not supersede case records" {
    param($c) $c.Artifact.summary.differences = 0
} $false "summary accounting mismatch"

$resultFile = [System.IO.Path]::GetFullPath($ResultPath)
$null = New-Item -ItemType Directory -Path (Split-Path -Parent $resultFile) -Force
$report = [ordered]@{
    schemaVersion = 1; channel = "non-official-policy-self-test"; passed = $true; checks = @($checks)
}
[System.IO.File]::WriteAllText($resultFile,
    ($report | ConvertTo-Json -Depth 10) + [Environment]::NewLine,
    [System.Text.UTF8Encoding]::new($false))
Write-Host "[OK] Non-official policy self-test: $($checks.Count) scenarios passed"
