<#
.SYNOPSIS
    Lua Rust — Quality Gate
.DESCRIPTION
    Runs the full quality gate for the Rust workspace:
    format check, clippy lint, test suite, documentation, and security audit.
.PARAMETER SkipFmt
    Skip cargo fmt --check.
.PARAMETER SkipClippy
    Skip cargo clippy.
.PARAMETER SkipAudit
    Skip cargo audit (use when audit DB is not available).
.PARAMETER JsonOutput
    Output results as JSON to stdout.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_quality_gate.ps1
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_quality_gate.ps1 -SkipAudit
#>

param(
    [switch]$SkipFmt,
    [switch]$SkipClippy,
    [switch]$SkipAudit,
    [switch]$JsonOutput
)

$ErrorActionPreference = "Continue"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Resolve-Path (Join-Path $ScriptDir "..")

$Results = [ordered]@{
    Format       = $null
    Clippy       = $null
    Test         = $null
    Doc          = $null
    Audit        = $null
}
$GateStart = Get-Date

Write-Host "=== Rust Quality Gate ===" -ForegroundColor Cyan
Write-Host "  Project: $ProjectRoot"
Write-Host "  Time:    $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')`n"

Push-Location $ProjectRoot

try {
    # ── 1/5: Format Check ──────────────────────────────────────────
    if (-not $SkipFmt) {
        Write-Host "[1/5] Format Check (cargo fmt --check)" -ForegroundColor Yellow
        $fmtOutput = cargo fmt --check 2>&1
        $Results.Format = ($LASTEXITCODE -eq 0)
        $status = if ($Results.Format) { "PASS" } else { "FAIL" }
        $color  = if ($Results.Format) { "Green" } else { "Red" }
        Write-Host "  Result: $status" -ForegroundColor $color
        if (-not $Results.Format) {
            Write-Host "  $fmtOutput" -ForegroundColor Red
        }
    }
    else {
        Write-Host "[1/5] Format Check — SKIPPED" -ForegroundColor Gray
        $Results.Format = $true
    }

    # ── 2/5: Clippy Lint ────────────────────────────────────────────
    if (-not $SkipClippy) {
        Write-Host "`n[2/5] Clippy Lint (cargo clippy --workspace -- -D warnings)" -ForegroundColor Yellow
        $clippyOutput = cargo clippy --workspace -- -D warnings 2>&1
        $Results.Clippy = ($LASTEXITCODE -eq 0)
        $status = if ($Results.Clippy) { "PASS" } else { "FAIL" }
        $color  = if ($Results.Clippy) { "Green" } else { "Red" }
        Write-Host "  Result: $status" -ForegroundColor $color
        if (-not $Results.Clippy) {
            Write-Host "  $clippyOutput" -ForegroundColor Red
        }
    }
    else {
        Write-Host "`n[2/5] Clippy Lint — SKIPPED" -ForegroundColor Gray
        $Results.Clippy = $true
    }

    # ── 3/5: Test Suite ─────────────────────────────────────────────
    Write-Host "`n[3/5] Test Suite (cargo nextest run --workspace)" -ForegroundColor Yellow
    $testOutput = cargo nextest run --workspace 2>&1
    $Results.Test = ($LASTEXITCODE -eq 0)
    $status = if ($Results.Test) { "PASS" } else { "FAIL" }
    $color  = if ($Results.Test) { "Green" } else { "Red" }
    Write-Host "  Result: $status" -ForegroundColor $color
    if (-not $Results.Test) {
        # Show only the summary lines to avoid drowning in output
        $testOutput | Select-String -Pattern "FAIL|PASS|Summary|error" | ForEach-Object {
            Write-Host "  $_" -ForegroundColor Red
        }
    }

    # ── 4/5: Documentation ──────────────────────────────────────────
    Write-Host "`n[4/5] Documentation (cargo doc --no-deps)" -ForegroundColor Yellow
    $docOutput = cargo doc --no-deps 2>&1
    $Results.Doc = ($LASTEXITCODE -eq 0)
    $status = if ($Results.Doc) { "PASS" } else { "FAIL" }
    $color  = if ($Results.Doc) { "Green" } else { "Red" }
    Write-Host "  Result: $status" -ForegroundColor $color

    # ── 5/5: Security Audit ─────────────────────────────────────────
    if (-not $SkipAudit) {
        Write-Host "`n[5/5] Security Audit (cargo audit)" -ForegroundColor Yellow
        $auditOutput = cargo audit 2>&1
        $auditExit = $LASTEXITCODE
        # cargo audit exits 0 on success, non-zero on vulnerabilities found
        $Results.Audit = ($auditExit -eq 0)
        $status = if ($Results.Audit) { "PASS" } else { "FAIL (vulnerabilities found)" }
        $color  = if ($Results.Audit) { "Green" } else { "Red" }
        Write-Host "  Result: $status" -ForegroundColor $color
    }
    else {
        Write-Host "`n[5/5] Security Audit — SKIPPED" -ForegroundColor Gray
        $Results.Audit = $true
    }
}
finally {
    Pop-Location
}

# ── Summary ─────────────────────────────────────────────────────────
$GateDuration = (Get-Date) - $GateStart
Write-Host "`n=== Quality Gate Summary ===" -ForegroundColor Cyan
Write-Host "  Duration: $($GateDuration.TotalSeconds.ToString('0.0'))s`n"

$ExitCode = 0
foreach ($key in $Results.Keys) {
    $status = if ($Results[$key]) { "PASS" } else { "FAIL" }
    $color  = if ($Results[$key]) { "Green" } else { "Red" }
    Write-Host "  [$status] $key" -ForegroundColor $color
    if (-not $Results[$key]) { $ExitCode = 1 }
}

if ($ExitCode -eq 0) {
    Write-Host "`n  ALL GATES PASSED" -ForegroundColor Green
}
else {
    Write-Host "`n  SOME GATES FAILED — see details above" -ForegroundColor Red
}

# JSON output for CI consumption
if ($JsonOutput) {
    $Results | ConvertTo-Json -Compress | Write-Host
}

exit $ExitCode
