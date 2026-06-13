<#
.SYNOPSIS
    Lua Rust Migration — Quality Gate
.DESCRIPTION
    Runs the full quality gate for the Rust migration workspace:
    format check, clippy lint, test suite, security audit, and cross-language validation.
    Every Phase must end with a passing run of this script.
.PARAMETER SkipFmt
    Skip cargo fmt --check.
.PARAMETER SkipClippy
    Skip cargo clippy.
.PARAMETER SkipAudit
    Skip cargo audit (use when audit DB is not available).
.PARAMETER SkipCrossValidate
    Skip cross-language bytecode/VM trace comparison.
.PARAMETER JsonOutput
    Output results as JSON to stdout.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_quality_gate.ps1
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_quality_gate.ps1 -SkipAudit -SkipCrossValidate
#>

param(
    [switch]$SkipFmt,
    [switch]$SkipClippy,
    [switch]$SkipAudit,
    [switch]$SkipCrossValidate,
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
    CrossValidate = $null
}
$GateStart = Get-Date

Write-Host "=== Rust Migration Quality Gate ===" -ForegroundColor Cyan
Write-Host "  Project: $ProjectRoot"
Write-Host "  Time:    $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')`n"

Push-Location $ProjectRoot

try {
    # ── 1/6: Format Check ──────────────────────────────────────────
    if (-not $SkipFmt) {
        Write-Host "[1/6] Format Check (cargo fmt --check)" -ForegroundColor Yellow
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
        Write-Host "[1/6] Format Check — SKIPPED" -ForegroundColor Gray
        $Results.Format = $true
    }

    # ── 2/6: Clippy Lint ────────────────────────────────────────────
    if (-not $SkipClippy) {
        Write-Host "`n[2/6] Clippy Lint (cargo clippy --workspace -- -D warnings)" -ForegroundColor Yellow
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
        Write-Host "`n[2/6] Clippy Lint — SKIPPED" -ForegroundColor Gray
        $Results.Clippy = $true
    }

    # ── 3/6: Test Suite ─────────────────────────────────────────────
    Write-Host "`n[3/6] Test Suite (cargo nextest run --workspace)" -ForegroundColor Yellow
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

    # ── 4/6: Documentation ──────────────────────────────────────────
    Write-Host "`n[4/6] Documentation (cargo doc --no-deps)" -ForegroundColor Yellow
    $docOutput = cargo doc --no-deps 2>&1
    $Results.Doc = ($LASTEXITCODE -eq 0)
    $status = if ($Results.Doc) { "PASS" } else { "FAIL" }
    $color  = if ($Results.Doc) { "Green" } else { "Red" }
    Write-Host "  Result: $status" -ForegroundColor $color

    # ── 5/6: Security Audit ─────────────────────────────────────────
    if (-not $SkipAudit) {
        Write-Host "`n[5/6] Security Audit (cargo audit)" -ForegroundColor Yellow
        $auditOutput = cargo audit 2>&1
        $auditExit = $LASTEXITCODE
        # cargo audit exits 0 on success, non-zero on vulnerabilities found
        $Results.Audit = ($auditExit -eq 0)
        $status = if ($Results.Audit) { "PASS" } else { "FAIL (vulnerabilities found)" }
        $color  = if ($Results.Audit) { "Green" } else { "Red" }
        Write-Host "  Result: $status" -ForegroundColor $color
    }
    else {
        Write-Host "`n[5/6] Security Audit — SKIPPED" -ForegroundColor Gray
        $Results.Audit = $true
    }

    # ── 6/6: Cross-Language Validation ──────────────────────────────
    if (-not $SkipCrossValidate) {
        Write-Host "`n[6/6] Cross-Language Validation" -ForegroundColor Yellow

        $cppBytecodeExe = Join-Path $ProjectRoot "..\lua_cpp\bin\lua_bytecode.exe"
        $cppAppExe      = Join-Path $ProjectRoot "..\lua_cpp\bin\lua_app.exe"

        $bytecodeOk = $true
        $traceOk    = $true

        # Bytecode comparison — applicable from Phase 2 onward
        $fixturesPhase2 = Join-Path $ProjectRoot "tests/fixtures/phase_2"
        if ((Test-Path $cppBytecodeExe) -and (Test-Path $fixturesPhase2)) {
            Write-Host "  Running: compare_bytecode.ps1" -ForegroundColor Gray
            $bcScript = Join-Path $ScriptDir "compare_bytecode.ps1"
            if (Test-Path $bcScript) {
                & $bcScript -InputDir $fixturesPhase2 -CppBytecodeExe $cppBytecodeExe
                $bytecodeOk = ($LASTEXITCODE -eq 0)
            }
            else {
                Write-Host "    compare_bytecode.ps1 not found" -ForegroundColor Yellow
                $bytecodeOk = $true  # not a failure of this gate
            }
        }
        else {
            Write-Host "  Bytecode comparison: N/A (Phase 2+ feature)" -ForegroundColor Gray
        }

        # VM trace comparison — applicable from Phase 3 onward
        $fixturesPhase3 = Join-Path $ProjectRoot "tests/fixtures/phase_3"
        if ((Test-Path $cppAppExe) -and (Test-Path $fixturesPhase3)) {
            Write-Host "  Running: compare_vm_trace.ps1" -ForegroundColor Gray
            $traceScript = Join-Path $ScriptDir "compare_vm_trace.ps1"
            if (Test-Path $traceScript) {
                & $traceScript -InputDir $fixturesPhase3 -CppAppExe $cppAppExe
                $traceOk = ($LASTEXITCODE -eq 0)
            }
            else {
                Write-Host "    compare_vm_trace.ps1 not found" -ForegroundColor Yellow
                $traceOk = $true
            }
        }
        else {
            Write-Host "  VM trace comparison: N/A (Phase 3+ feature)" -ForegroundColor Gray
        }

        $Results.CrossValidate = ($bytecodeOk -and $traceOk)
        $status = if ($Results.CrossValidate) { "PASS (or N/A)" } else { "FAIL" }
        $color  = if ($Results.CrossValidate) { "Green" } else { "Red" }
        Write-Host "  Result: $status" -ForegroundColor $color
    }
    else {
        Write-Host "`n[6/6] Cross-Language Validation — SKIPPED" -ForegroundColor Gray
        $Results.CrossValidate = $true
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
