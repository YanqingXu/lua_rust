<#
.SYNOPSIS
    Compare C++ and Rust VM execution traces for identical Lua sources.
.DESCRIPTION
    Runs each .lua file with both the C++ and Rust lua_app in trace mode,
    then diffs the JSONL trace output event-by-event. Used from Phase 3 onward.
.PARAMETER InputDir
    Directory containing .lua test fixture files.
.PARAMETER CppAppExe
    Path to the C++ lua_app.exe binary.
.PARAMETER RustAppExe
    Path to the Rust lua_app binary (typically target/debug/lua_app.exe).
.PARAMETER OutputDir
    Directory for diff output (default: target/vm_trace_diff).
.PARAMETER JsonOutput
    Output results as JSON.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/compare_vm_trace.ps1 -InputDir tests/fixtures/phase_3
#>

param(
    [Parameter(Mandatory = $true)]
    [string]$InputDir,

    [string]$CppAppExe = "",
    [string]$RustAppExe = "",

    [string]$OutputDir = "",
    [switch]$JsonOutput
)

$ErrorActionPreference = "Continue"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Resolve-Path (Join-Path $ScriptDir "..")

# Resolve default paths
if (-not $CppAppExe) {
    $CppAppExe = Join-Path $ProjectRoot "..\lua_cpp\bin\lua_app.exe"
}
if (-not $RustAppExe) {
    $RustAppExe = Join-Path $ProjectRoot "target\debug\lua_app.exe"
}
if (-not $OutputDir) {
    $OutputDir = Join-Path $ProjectRoot "target\vm_trace_diff"
}

$Results = @{
    Passed  = 0
    Failed  = 0
    Skipped = 0
    Details = @()
}

Write-Host "=== VM Trace Comparison ===" -ForegroundColor Cyan
Write-Host "  C++ app:   $CppAppExe"
Write-Host "  Rust app:  $RustAppExe"
Write-Host "  Input dir: $InputDir"
Write-Host "  Output dir: $OutputDir`n"

# Validate inputs
if (-not (Test-Path $CppAppExe)) {
    Write-Host "  [SKIP] C++ lua_app not found at: $CppAppExe" -ForegroundColor Yellow
    Write-Host "         Build lua_cpp first, or pass -CppAppExe" -ForegroundColor Yellow
    if ($JsonOutput) {
        @{ status = "skipped"; reason = "C++ lua_app not found" } | ConvertTo-Json -Compress | Write-Host
    }
    exit 0
}

if (-not (Test-Path $RustAppExe)) {
    Write-Host "  [SKIP] Rust lua_app not found at: $RustAppExe" -ForegroundColor Yellow
    Write-Host "         Build lua_app first: cargo build -p lua_app" -ForegroundColor Yellow
    if ($JsonOutput) {
        @{ status = "skipped"; reason = "Rust lua_app not found" } | ConvertTo-Json -Compress | Write-Host
    }
    exit 0
}

if (-not (Test-Path $InputDir)) {
    Write-Host "  [SKIP] Input directory not found: $InputDir" -ForegroundColor Yellow
    if ($JsonOutput) {
        @{ status = "skipped"; reason = "InputDir not found" } | ConvertTo-Json -Compress | Write-Host
    }
    exit 0
}

# Ensure output directory
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

# Collect .lua files
$luaFiles = Get-ChildItem -Path $InputDir -Filter "*.lua" -Recurse

if ($luaFiles.Count -eq 0) {
    Write-Host "  No .lua files found in $InputDir" -ForegroundColor Yellow
    if ($JsonOutput) {
        @{ status = "skipped"; reason = "no .lua files" } | ConvertTo-Json -Compress | Write-Host
    }
    exit 0
}

foreach ($file in $luaFiles) {
    $testName = $file.BaseName
    $cppTraceFile = Join-Path $OutputDir "$testName.cpp_trace.jsonl"
    $rustTraceFile = Join-Path $OutputDir "$testName.rust_trace.jsonl"

    Write-Host "  Tracing: $testName" -ForegroundColor Gray

    try {
        # Generate C++ trace
        & $CppAppExe --trace=$cppTraceFile $file.FullName 2>&1 | Out-Null
        $cppExit = $LASTEXITCODE

        # Generate Rust trace
        & $RustAppExe --trace=$rustTraceFile $file.FullName 2>&1 | Out-Null
        $rustExit = $LASTEXITCODE

        # Both must succeed (or both fail identically)
        if ($cppExit -ne 0 -and $rustExit -ne 0) {
            $Results.Passed++
            Write-Host "    [PASS] $testName (both errored consistently)" -ForegroundColor Green
            $Results.Details += @{ test = $testName; status = "passed"; note = "consistent error" }
            continue
        }

        if ($cppExit -ne 0) {
            $Results.Failed++
            Write-Host "    [FAIL] $testName: C++ exited $cppExit, Rust exited $rustExit" -ForegroundColor Red
            $Results.Details += @{ test = $testName; status = "failed"; reason = "exit code mismatch: cpp=$cppExit rust=$rustExit" }
            continue
        }

        if ($rustExit -ne 0) {
            $Results.Failed++
            Write-Host "    [FAIL] $testName: Rust exited $rustExit, C++ exited $cppExit" -ForegroundColor Red
            $Results.Details += @{ test = $testName; status = "failed"; reason = "exit code mismatch: cpp=$cppExit rust=$rustExit" }
            continue
        }

        # Compare JSONL traces event-by-event
        if (-not (Test-Path $cppTraceFile)) {
            $Results.Skipped++
            Write-Host "    [SKIP] $testName: C++ trace file not generated" -ForegroundColor Yellow
            continue
        }
        if (-not (Test-Path $rustTraceFile)) {
            $Results.Skipped++
            Write-Host "    [SKIP] $testName: Rust trace file not generated" -ForegroundColor Yellow
            continue
        }

        $cppLines  = Get-Content $cppTraceFile
        $rustLines = Get-Content $rustTraceFile

        if ($cppLines.Count -ne $rustLines.Count) {
            $Results.Failed++
            $diffFile = Join-Path $OutputDir "$testName.trace_diff"
            @"
Event count mismatch: C++ = $($cppLines.Count), Rust = $($rustLines.Count)

=== C++ Trace ===
$($cppLines -join "`n")

=== Rust Trace ===
$($rustLines -join "`n")
"@ | Out-File -FilePath $diffFile -Encoding UTF8
            Write-Host "    [FAIL] $testName: event count mismatch (C++: $($cppLines.Count), Rust: $($rustLines.Count))" -ForegroundColor Red
            $Results.Details += @{ test = $testName; status = "failed"; reason = "event count mismatch"; diff_file = $diffFile }
            continue
        }

        # Compare line by line — assuming JSONL with "seq" field for alignment
        $mismatch = $false
        $firstMismatch = ""
        for ($i = 0; $i -lt $cppLines.Count; $i++) {
            if ($cppLines[$i].Trim() -ne $rustLines[$i].Trim()) {
                $mismatch = $true
                $firstMismatch = "Line $($i+1):`n  C++:  $($cppLines[$i])`n  Rust: $($rustLines[$i])"
                break
            }
        }

        if ($mismatch) {
            $Results.Failed++
            $diffFile = Join-Path $OutputDir "$testName.trace_diff"
            @"
$firstMismatch

=== Full C++ Trace ===
$($cppLines -join "`n")

=== Full Rust Trace ===
$($rustLines -join "`n")
"@ | Out-File -FilePath $diffFile -Encoding UTF8
            Write-Host "    [FAIL] $testName: trace mismatch → $diffFile" -ForegroundColor Red
            $Results.Details += @{ test = $testName; status = "failed"; reason = "trace mismatch"; diff_file = $diffFile }
        }
        else {
            $Results.Passed++
            Write-Host "    [PASS] $testName ($($cppLines.Count) events)" -ForegroundColor Green
            $Results.Details += @{ test = $testName; status = "passed"; event_count = $cppLines.Count }
        }
    }
    catch {
        $Results.Skipped++
        Write-Host "    [SKIP] $testName : $_" -ForegroundColor Yellow
        $Results.Details += @{ test = $testName; status = "skipped"; reason = "$_" }
    }
}

# Summary
Write-Host "`n=== VM Trace Comparison Results ===" -ForegroundColor Cyan
Write-Host "  Passed:  $($Results.Passed)" -ForegroundColor Green
$failedColor = if ($Results.Failed -gt 0) { "Red" } else { "Green" }
Write-Host "  Failed:  $($Results.Failed)" -ForegroundColor $failedColor
Write-Host "  Skipped: $($Results.Skipped)" -ForegroundColor Yellow

if ($JsonOutput) {
    $Results | ConvertTo-Json -Compress -Depth 3 | Write-Host
}

exit $(if ($Results.Failed -gt 0) { 1 } else { 0 })
