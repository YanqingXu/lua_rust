<#
.SYNOPSIS
    Lua Rust — Quality Gate
.DESCRIPTION
    Runs the full quality gate for the Rust workspace:
    format check, all-targets clippy lint, Debug and Release test suites,
    warning-free documentation, and security audit.
.PARAMETER SkipFmt
    Skip cargo fmt --check. Requires AllowSkipped or Smoke for a successful
    partial local exit.
.PARAMETER SkipClippy
    Skip cargo clippy. Requires AllowSkipped or Smoke for a successful partial
    local exit.
.PARAMETER SkipAudit
    Skip cargo audit when it is unavailable. Requires AllowSkipped or Smoke for
    a successful partial local exit.
.PARAMETER JsonOutput
    Output results as JSON to stdout.
.PARAMETER AllowSkipped
    Permit an explicitly partial local run to return success. The run is still
    reported as partial and never as a full gate pass.
.PARAMETER Smoke
    Mark the invocation as a local smoke run. Smoke mode permits skipped checks
    to return success but never reports a full gate pass.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_quality_gate.ps1
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_quality_gate.ps1 -SkipAudit -Smoke
#>

param(
    [switch]$SkipFmt,
    [switch]$SkipClippy,
    [switch]$SkipAudit,
    [switch]$JsonOutput,
    [switch]$AllowSkipped,
    [switch]$Smoke
)

$ErrorActionPreference = "Continue"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Resolve-Path (Join-Path $ScriptDir "..")

$Results = [ordered]@{
    Format      = "NOT_RUN"
    Clippy      = "NOT_RUN"
    TestDebug   = "NOT_RUN"
    TestRelease = "NOT_RUN"
    Doc         = "NOT_RUN"
    Audit       = "NOT_RUN"
}
$GateStart = Get-Date

function Invoke-Cargo {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    $output = @(& cargo @Arguments 2>&1)
    return [pscustomobject]@{
        ExitCode = $LASTEXITCODE
        Output   = $output
    }
}

function Write-FailureOutput {
    param(
        [Parameter(Mandatory = $true)]
        [object[]]$Output
    )

    $Output | Select-Object -Last 100 | ForEach-Object {
        Write-Host "  $_" -ForegroundColor Red
    }
}

Write-Host "=== Rust Quality Gate ===" -ForegroundColor Cyan
Write-Host "  Project: $ProjectRoot"
Write-Host "  Time:    $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')`n"

Push-Location $ProjectRoot

try {
    # ── 1/6: Format Check ──────────────────────────────────────────
    if (-not $SkipFmt) {
        Write-Host "[1/6] Format Check (cargo fmt --check)" -ForegroundColor Yellow
        $fmt = Invoke-Cargo -Arguments @("fmt", "--check")
        $Results.Format = if ($fmt.ExitCode -eq 0) { "PASS" } else { "FAIL" }
        $color = if ($Results.Format -eq "PASS") { "Green" } else { "Red" }
        Write-Host "  Result: $($Results.Format)" -ForegroundColor $color
        if ($Results.Format -eq "FAIL") {
            Write-FailureOutput -Output $fmt.Output
        }
    }
    else {
        Write-Host "[1/6] Format Check -- SKIPPED" -ForegroundColor Gray
        $Results.Format = "SKIPPED"
    }

    # ── 2/6: Clippy Lint ────────────────────────────────────────────
    if (-not $SkipClippy) {
        Write-Host "`n[2/6] Clippy Lint (cargo clippy --workspace --all-targets -- -D warnings)" -ForegroundColor Yellow
        $clippy = Invoke-Cargo -Arguments @(
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings"
        )
        $Results.Clippy = if ($clippy.ExitCode -eq 0) { "PASS" } else { "FAIL" }
        $color = if ($Results.Clippy -eq "PASS") { "Green" } else { "Red" }
        Write-Host "  Result: $($Results.Clippy)" -ForegroundColor $color
        if ($Results.Clippy -eq "FAIL") {
            Write-FailureOutput -Output $clippy.Output
        }
    }
    else {
        Write-Host "`n[2/6] Clippy Lint -- SKIPPED" -ForegroundColor Gray
        $Results.Clippy = "SKIPPED"
    }

    # ── 3/6: Debug and Release Test Suites ─────────────────────────
    $hasNextest = $null -ne (Get-Command "cargo-nextest" -ErrorAction SilentlyContinue)
    if ($hasNextest) {
        $debugTestArguments = @("nextest", "run", "--workspace")
        $releaseTestArguments = @("nextest", "run", "--workspace", "--release")
        $testRunner = "cargo nextest"
    }
    else {
        $debugTestArguments = @("test", "--workspace")
        $releaseTestArguments = @("test", "--workspace", "--release")
        $testRunner = "cargo test (cargo-nextest not installed)"
    }

    Write-Host "`n[3/6] Debug Tests ($testRunner)" -ForegroundColor Yellow
    $debugTests = Invoke-Cargo -Arguments $debugTestArguments
    $Results.TestDebug = if ($debugTests.ExitCode -eq 0) { "PASS" } else { "FAIL" }
    $color = if ($Results.TestDebug -eq "PASS") { "Green" } else { "Red" }
    Write-Host "  Result: $($Results.TestDebug)" -ForegroundColor $color
    if ($Results.TestDebug -eq "FAIL") {
        Write-FailureOutput -Output $debugTests.Output
    }

    Write-Host "`n[4/6] Release Tests ($testRunner --release)" -ForegroundColor Yellow
    $releaseTests = Invoke-Cargo -Arguments $releaseTestArguments
    $Results.TestRelease = if ($releaseTests.ExitCode -eq 0) { "PASS" } else { "FAIL" }
    $color = if ($Results.TestRelease -eq "PASS") { "Green" } else { "Red" }
    Write-Host "  Result: $($Results.TestRelease)" -ForegroundColor $color
    if ($Results.TestRelease -eq "FAIL") {
        Write-FailureOutput -Output $releaseTests.Output
    }

    # ── 5/6: Documentation ──────────────────────────────────────────
    Write-Host "`n[5/6] Documentation (RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps)" -ForegroundColor Yellow
    $previousRustdocFlags = $env:RUSTDOCFLAGS
    try {
        $env:RUSTDOCFLAGS = "-D warnings"
        $doc = Invoke-Cargo -Arguments @("doc", "--workspace", "--no-deps")
    }
    finally {
        $env:RUSTDOCFLAGS = $previousRustdocFlags
    }
    $Results.Doc = if ($doc.ExitCode -eq 0) { "PASS" } else { "FAIL" }
    $color = if ($Results.Doc -eq "PASS") { "Green" } else { "Red" }
    Write-Host "  Result: $($Results.Doc)" -ForegroundColor $color
    if ($Results.Doc -eq "FAIL") {
        Write-FailureOutput -Output $doc.Output
    }

    # ── 6/6: Security Audit ─────────────────────────────────────────
    if (-not $SkipAudit) {
        Write-Host "`n[6/6] Security Audit (cargo audit --json)" -ForegroundColor Yellow
        if ($null -eq (Get-Command "cargo-audit" -ErrorAction SilentlyContinue)) {
            $Results.Audit = "TOOL_MISSING"
            Write-Host "  Result: TOOL_MISSING" -ForegroundColor Red
            Write-Host "  Install cargo-audit, or pass -SkipAudit for an explicitly skipped local audit." -ForegroundColor Red
        }
        else {
            $audit = Invoke-Cargo -Arguments @("audit", "--json")
            if ($audit.ExitCode -eq 0) {
                $Results.Audit = "PASS"
            }
            else {
                $auditJson = $null
                foreach ($line in $audit.Output) {
                    try {
                        $candidate = "$line" | ConvertFrom-Json -ErrorAction Stop
                        if ($null -ne $candidate.vulnerabilities) {
                            $auditJson = $candidate
                            break
                        }
                    }
                    catch {
                        # Non-JSON cargo diagnostics are classified below.
                    }
                }

                if (($null -ne $auditJson) -and $auditJson.vulnerabilities.found) {
                    $Results.Audit = "VULNERABILITIES"
                }
                else {
                    $auditText = $audit.Output -join "`n"
                    $networkPattern = "network|failed to fetch|failed to update|could not resolve|connection|timed out|TLS|SSL|HTTP"
                    $Results.Audit = if ($auditText -match $networkPattern) {
                        "NETWORK_ERROR"
                    }
                    else {
                        "AUDIT_ERROR"
                    }
                }
            }

            $color = if ($Results.Audit -eq "PASS") { "Green" } else { "Red" }
            Write-Host "  Result: $($Results.Audit)" -ForegroundColor $color
            if ($Results.Audit -ne "PASS") {
                Write-FailureOutput -Output $audit.Output
            }
        }
    }
    else {
        Write-Host "`n[6/6] Security Audit -- SKIPPED" -ForegroundColor Gray
        $Results.Audit = "SKIPPED"
    }
}
finally {
    Pop-Location
}

# ── Summary ─────────────────────────────────────────────────────────
$GateDuration = (Get-Date) - $GateStart
Write-Host "`n=== Quality Gate Summary ===" -ForegroundColor Cyan
Write-Host "  Duration: $($GateDuration.TotalSeconds.ToString('0.0'))s`n"

$hasFailures = $false
$hasSkipped = $false
foreach ($key in $Results.Keys) {
    $status = $Results[$key]
    $color = if ($status -eq "PASS") {
        "Green"
    }
    elseif ($status -eq "SKIPPED") {
        "Gray"
    }
    else {
        "Red"
    }
    Write-Host "  [$status] $key" -ForegroundColor $color
    if ($status -eq "SKIPPED") {
        $hasSkipped = $true
    } elseif ($status -ne "PASS") {
        $hasFailures = $true
    }
}

$fullPassed = -not $hasFailures -and -not $hasSkipped -and -not $Smoke
$checksPassed = -not $hasFailures
$partialSuccessAllowed = $AllowSkipped -or $Smoke
$ExitCode = if (
    $fullPassed -or
    ($checksPassed -and $hasSkipped -and $partialSuccessAllowed) -or
    ($checksPassed -and $Smoke)
) {
    0
} else {
    1
}

if ($fullPassed) {
    Write-Host "`n  ALL GATES PASSED" -ForegroundColor Green
} elseif ($checksPassed -and $Smoke) {
    Write-Host (
        "`n  SMOKE CHECKS PASSED -- this is not a full quality-gate pass"
    ) -ForegroundColor Yellow
} elseif ($checksPassed -and $hasSkipped -and $AllowSkipped) {
    Write-Host (
        "`n  PARTIAL CHECKS PASSED -- skipped checks prevent a full pass"
    ) -ForegroundColor Yellow
} elseif ($checksPassed -and $hasSkipped) {
    Write-Host (
        "`n  GATE INCOMPLETE -- rerun without skips or use an explicit local mode"
    ) -ForegroundColor Red
}
else {
    Write-Host "`n  SOME GATES FAILED -- see details above" -ForegroundColor Red
}

# JSON output for CI consumption
if ($JsonOutput) {
    [ordered]@{
        mode = if ($Smoke) {
            "smoke"
        } elseif ($AllowSkipped) {
            "allow-skipped"
        } else {
            "full"
        }
        checksPassed = $checksPassed
        fullPassed = $fullPassed
        hasSkipped = $hasSkipped
        results = $Results
    } | ConvertTo-Json -Compress | Write-Host
}

exit $ExitCode
