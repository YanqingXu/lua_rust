<#
.SYNOPSIS
    Independently exercise the bytecode and VM trace parity runner infrastructure.
.DESCRIPTION
    Bytecode uses the same C++ dumper on both sides. VM trace uses the deterministic
    process fixture because current production trace support is itself under parity
    evaluation. These checks prove runner plumbing only and never claim language parity.
#>

[CmdletBinding()]
param(
    [string]$InputPath = "tests/lua/bytecode/test_bytecode.lua",
    [string]$CppBytecodeExe = "",
    [string]$OutputDir = "target/parity/runner-self-test"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDirectory ".."))
$bytecodeRunner = Join-Path $scriptDirectory "compare_bytecode.ps1"
$traceRunner = Join-Path $scriptDirectory "compare_vm_trace.ps1"

if (-not [System.IO.Path]::IsPathRooted($InputPath)) {
    $InputPath = Join-Path $projectRoot $InputPath
}
if (-not [System.IO.Path]::IsPathRooted($OutputDir)) {
    $OutputDir = Join-Path $projectRoot $OutputDir
}
if (-not $CppBytecodeExe) {
    $CppBytecodeExe = Join-Path $projectRoot "..\lua_cpp\bin\lua_bytecode.exe"
}
elseif (-not [System.IO.Path]::IsPathRooted($CppBytecodeExe)) {
    $CppBytecodeExe = Join-Path $projectRoot $CppBytecodeExe
}

$bytecodeOutput = Join-Path $OutputDir "bytecode"
$bytecodeDirectoryOutput = Join-Path $OutputDir "bytecode-directory-representative"
$traceOutput = Join-Path $OutputDir "vm-trace"

& $bytecodeRunner -InputPath $InputPath -CppBytecodeExe $CppBytecodeExe `
    -InfrastructureSelfTest -SelfTestTool Cpp -OutputDir $bytecodeOutput
$bytecodeExit = $LASTEXITCODE
if ($bytecodeExit -ne 0) {
    throw "bytecode runner self-test failed with exit code $bytecodeExit"
}

$bytecodeDirectoryInput = Split-Path -Parent $InputPath
& $bytecodeRunner -InputPath $bytecodeDirectoryInput -CorpusMode Representative `
    -RepresentativeCount 1 -CppBytecodeExe $CppBytecodeExe `
    -InfrastructureSelfTest -SelfTestTool Cpp -OutputDir $bytecodeDirectoryOutput
$bytecodeDirectoryExit = $LASTEXITCODE
if ($bytecodeDirectoryExit -ne 0) {
    throw "bytecode directory representative self-test failed with exit code $bytecodeDirectoryExit"
}

& $traceRunner -InputPath $InputPath -InfrastructureSelfTest -SelfTestTool Synthetic `
    -OutputDir $traceOutput
$traceExit = $LASTEXITCODE
if ($traceExit -ne 0) {
    throw "VM trace runner self-test failed with exit code $traceExit"
}

$bytecodeReportPath = Join-Path $bytecodeOutput "report.json"
$bytecodeDirectoryReportPath = Join-Path $bytecodeDirectoryOutput "report.json"
$traceReportPath = Join-Path $traceOutput "report.json"
$bytecodeReport = Get-Content -LiteralPath $bytecodeReportPath -Raw | ConvertFrom-Json
$bytecodeDirectoryReport = Get-Content -LiteralPath $bytecodeDirectoryReportPath -Raw | ConvertFrom-Json
$traceReport = Get-Content -LiteralPath $traceReportPath -Raw | ConvertFrom-Json

foreach ($item in @(
    [pscustomobject]@{ name = "bytecode"; report = $bytecodeReport },
    [pscustomobject]@{ name = "bytecode-directory-representative"; report = $bytecodeDirectoryReport },
    [pscustomobject]@{ name = "vm-trace"; report = $traceReport }
)) {
    if ($item.report.purpose -ne "infrastructure-self-test" -or
        $item.report.status -ne "self-test-passed" -or
        $item.report.summary.selected -lt 1 -or
        $item.report.summary.failed -ne 0) {
        throw "$($item.name) report did not record a clean infrastructure-only self-test"
    }
    $execution = $item.report.cases[0].executions.left
    if ($null -eq $execution.command -or
        $null -eq $execution.stdout -or
        $null -eq $execution.stderr -or
        $null -eq $execution.exitCode -or
        $null -eq $execution.timedOut) {
        throw "$($item.name) report is missing required process evidence"
    }
}

if ($bytecodeDirectoryReport.summary.selected -ne 1) {
    throw "bytecode directory representative self-test did not select exactly one case"
}

foreach ($evidence in $traceReport.requiredEvidence) {
    if (-not [bool]$traceReport.cases[0].evidence.left.$evidence) {
        throw "VM trace self-test fixture did not cover required evidence '$evidence'"
    }
}

$summary = [ordered]@{
    schemaVersion = 1
    status = "passed"
    bytecodeReport = $bytecodeReportPath
    bytecodeDirectoryReport = $bytecodeDirectoryReportPath
    vmTraceReport = $traceReportPath
}
$summary | ConvertTo-Json -Depth 8
exit 0
