<#
.SYNOPSIS
    Compare C++ and Rust compiler bytecode output for identical Lua sources.
.DESCRIPTION
    Compiles each .lua file in InputDir with both the C++ and Rust bytecode
    tools, then diffs the output. Failures are saved to target/bytecode_diff/.
    Used from Phase 2 onward to validate compiler alignment.
.PARAMETER InputDir
    Directory containing .lua test fixture files.
.PARAMETER CppBytecodeExe
    Path to the C++ lua_bytecode.exe binary.
.PARAMETER RustBytecodeExe
    Path to the Rust lua_bytecode binary (typically target/debug/lua_bytecode.exe).
.PARAMETER OutputDir
    Directory for diff output (default: target/bytecode_diff).
.PARAMETER JsonOutput
    Output results as JSON.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/compare_bytecode.ps1 -InputDir tests/fixtures/phase_2
#>

param(
    [Parameter(Mandatory = $true)]
    [string]$InputDir,

    [string]$CppBytecodeExe = "",
    [string]$RustBytecodeExe = "",

    [string]$OutputDir = "",
    [switch]$JsonOutput
)

$ErrorActionPreference = "Continue"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Resolve-Path (Join-Path $ScriptDir "..")

# Resolve default paths
if (-not $CppBytecodeExe) {
    $CppBytecodeExe = Join-Path $ProjectRoot "..\lua_cpp\bin\lua_bytecode.exe"
}
if (-not $RustBytecodeExe) {
    $RustBytecodeExe = Join-Path $ProjectRoot "target\debug\lua_bytecode.exe"
}
if (-not $OutputDir) {
    $OutputDir = Join-Path $ProjectRoot "target\bytecode_diff"
}

$Results = @{
    Passed  = 0
    Failed  = 0
    Skipped = 0
    Details = @()
}

Write-Host "=== Bytecode Comparison ===" -ForegroundColor Cyan
Write-Host "  C++ tool:   $CppBytecodeExe"
Write-Host "  Rust tool:  $RustBytecodeExe"
Write-Host "  Input dir:  $InputDir"
Write-Host "  Output dir: $OutputDir`n"

# Validate inputs
if (-not (Test-Path $CppBytecodeExe)) {
    Write-Host "  [SKIP] C++ bytecode tool not found at: $CppBytecodeExe" -ForegroundColor Yellow
    Write-Host "         Build lua_cpp first, or pass -CppBytecodeExe" -ForegroundColor Yellow
    if ($JsonOutput) {
        @{ status = "skipped"; reason = "C++ tool not found" } | ConvertTo-Json -Compress | Write-Host
    }
    exit 0
}

if (-not (Test-Path $RustBytecodeExe)) {
    Write-Host "  [SKIP] Rust bytecode tool not found at: $RustBytecodeExe" -ForegroundColor Yellow
    Write-Host "         Build lua_bytecode first: cargo build -p lua_bytecode" -ForegroundColor Yellow
    if ($JsonOutput) {
        @{ status = "skipped"; reason = "Rust tool not found" } | ConvertTo-Json -Compress | Write-Host
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
    $relPath = Resolve-Path -Relative $file.FullName
    Write-Host "  Comparing: $testName" -ForegroundColor Gray

    try {
        # Generate C++ bytecode output (JSON format for structural comparison)
        $cppOut = & $CppBytecodeExe $file.FullName --format=json 2>&1 | Out-String
        $cppExit = $LASTEXITCODE

        # Generate Rust bytecode output
        $rustOut = & $RustBytecodeExe $file.FullName --format=json 2>&1 | Out-String
        $rustExit = $LASTEXITCODE

        # Both must succeed
        if ($cppExit -ne 0) {
            $Results.Skipped++
            $reason = "C++ tool exited with code $cppExit"
            Write-Host "    [SKIP] $testName: $reason" -ForegroundColor Yellow
            $Results.Details += @{ test = $testName; status = "skipped"; reason = $reason }
            continue
        }
        if ($rustExit -ne 0) {
            $Results.Failed++
            $reason = "Rust tool exited with code $rustExit"
            Write-Host "    [FAIL] $testName: $reason" -ForegroundColor Red
            $Results.Details += @{ test = $testName; status = "failed"; reason = $reason }
            continue
        }

        # Compare outputs (structural comparison)
        if ($cppOut.Trim() -eq $rustOut.Trim()) {
            $Results.Passed++
            Write-Host "    [PASS] $testName" -ForegroundColor Green
            $Results.Details += @{ test = $testName; status = "passed" }
        }
        else {
            $Results.Failed++
            $diffFile = Join-Path $OutputDir "$testName.diff"
            @"
=== C++ Output ($relPath) ===
$cppOut

=== Rust Output ($relPath) ===
$rustOut
"@ | Out-File -FilePath $diffFile -Encoding UTF8

            Write-Host "    [FAIL] $testName → $diffFile" -ForegroundColor Red
            $Results.Details += @{ test = $testName; status = "failed"; diff_file = $diffFile }
        }
    }
    catch {
        $Results.Skipped++
        Write-Host "    [SKIP] $testName : $_" -ForegroundColor Yellow
        $Results.Details += @{ test = $testName; status = "skipped"; reason = "$_" }
    }
}

# Summary
Write-Host "`n=== Bytecode Comparison Results ===" -ForegroundColor Cyan
Write-Host "  Passed:  $($Results.Passed)" -ForegroundColor Green
$failedColor = if ($Results.Failed -gt 0) { "Red" } else { "Green" }
Write-Host "  Failed:  $($Results.Failed)" -ForegroundColor $failedColor
Write-Host "  Skipped: $($Results.Skipped)" -ForegroundColor Yellow

if ($JsonOutput) {
    $Results | ConvertTo-Json -Compress -Depth 3 | Write-Host
}

exit $(if ($Results.Failed -gt 0) { 1 } else { 0 })
