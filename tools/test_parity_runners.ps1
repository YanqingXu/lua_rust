<#
.SYNOPSIS
    Independently exercise the bytecode and VM trace parity runner infrastructure.
.DESCRIPTION
    Bytecode uses the same C++ dumper on both sides. VM trace exercises the deterministic
    complete-profile fixture plus the real C++ and Rust basic snapshot/diff producers in
    self-comparison mode. A real cross-language lifecycle fixture proves Complete-profile
    CALL/RETURN/YIELD/RESUME, Upvalue open/close, and error parity.
#>

[CmdletBinding()]
param(
    [string]$InputPath = "tests/lua/bytecode/test_bytecode.lua",
    [string]$CppAppExe = "",
    [string]$CppBytecodeExe = "",
    [string]$CppEvidenceExe = "",
    [string]$CppTraceExe = "",
    [string]$RustAppExe = "",
    [string]$RustBytecodeExe = "",
    [string]$OutputDir = "target/parity/runner-self-test"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDirectory ".."))
$bytecodeRunner = Join-Path $scriptDirectory "compare_bytecode.ps1"
$traceRunner = Join-Path $scriptDirectory "compare_vm_trace.ps1"
$traceStateRetirementRunner = Join-Path $scriptDirectory "test_vm_trace_state_retirement.ps1"
$cliContractRunner = Join-Path $scriptDirectory "test_cli_contract.ps1"
$replContractRunner = Join-Path $scriptDirectory "test_repl_contract.ps1"
$bytecodeToolContractRunner = Join-Path $scriptDirectory "test_bytecode_tool_contract.ps1"

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
if (-not $CppAppExe) {
    $cppAppName = if ([System.IO.Path]::GetExtension($CppBytecodeExe) -ieq ".exe") {
        "lua_app.exe"
    }
    else {
        "lua_app"
    }
    $CppAppExe = Join-Path (Split-Path -Parent $CppBytecodeExe) $cppAppName
}
elseif (-not [System.IO.Path]::IsPathRooted($CppAppExe)) {
    $CppAppExe = Join-Path $projectRoot $CppAppExe
}
if (-not $CppEvidenceExe) {
    $cppEvidenceName = if (
        [System.IO.Path]::GetExtension($CppBytecodeExe) -ieq ".exe"
    ) {
        "lua_bytecode_evidence.exe"
    }
    else {
        "lua_bytecode_evidence"
    }
    $CppEvidenceExe = Join-Path (Split-Path -Parent $CppBytecodeExe) $cppEvidenceName
}
elseif (-not [System.IO.Path]::IsPathRooted($CppEvidenceExe)) {
    $CppEvidenceExe = Join-Path $projectRoot $CppEvidenceExe
}
if (-not $CppTraceExe) {
    $traceName = if ([System.IO.Path]::GetExtension($CppBytecodeExe) -ieq ".exe") {
        "lua_vm_trace_evidence.exe"
    }
    else {
        "lua_vm_trace_evidence"
    }
    $CppTraceExe = Join-Path (Split-Path -Parent $CppBytecodeExe) $traceName
}
elseif (-not [System.IO.Path]::IsPathRooted($CppTraceExe)) {
    $CppTraceExe = Join-Path $projectRoot $CppTraceExe
}
if (-not $RustAppExe) {
    $rustName = if ($env:OS -eq "Windows_NT") { "lua_app.exe" } else { "lua_app" }
    $configured = Join-Path $projectRoot "target/x86_64-pc-windows-msvc/debug/$rustName"
    $hostTarget = Join-Path $projectRoot "target/debug/$rustName"
    $RustAppExe = if (Test-Path -LiteralPath $configured -PathType Leaf) {
        $configured
    }
    else {
        $hostTarget
    }
}
elseif (-not [System.IO.Path]::IsPathRooted($RustAppExe)) {
    $RustAppExe = Join-Path $projectRoot $RustAppExe
}
if (-not $RustBytecodeExe) {
    $rustBytecodeName = if ([System.IO.Path]::GetExtension($RustAppExe) -ieq ".exe") {
        "lua_bytecode.exe"
    }
    else {
        "lua_bytecode"
    }
    $RustBytecodeExe = Join-Path (Split-Path -Parent $RustAppExe) $rustBytecodeName
}
elseif (-not [System.IO.Path]::IsPathRooted($RustBytecodeExe)) {
    $RustBytecodeExe = Join-Path $projectRoot $RustBytecodeExe
}

$bytecodeOutput = Join-Path $OutputDir "bytecode"
$bytecodeDirectoryOutput = Join-Path $OutputDir "bytecode-directory-representative"
$bytecodeCppNulOutput = Join-Path $OutputDir "bytecode-cpp-nul-constant"
$bytecodeRustRecursiveOutput = Join-Path $OutputDir "bytecode-rust-recursive-singleton"
$bytecodeEmptyOutput = Join-Path $OutputDir "bytecode-empty-output"
$traceOutput = Join-Path $OutputDir "vm-trace"
$traceCppOutput = Join-Path $OutputDir "vm-trace-cpp-snapshot"
$traceRustOutput = Join-Path $OutputDir "vm-trace-rust-snapshot"
$traceCppDiffOutput = Join-Path $OutputDir "vm-trace-cpp-diff"
$traceRustDiffOutput = Join-Path $OutputDir "vm-trace-rust-diff"
$traceCompleteOutput = Join-Path $OutputDir "vm-trace-complete-lifecycle"
$traceCallbackDiffOutput = Join-Path $OutputDir "vm-trace-callback-diff"
$traceCallbackSnapshotOutput = Join-Path $OutputDir "vm-trace-callback-snapshot"
$traceStateRetirementCppOutput = Join-Path $OutputDir "vm-trace-state-retirement-cpp"
$traceStateRetirementRustOutput = Join-Path $OutputDir "vm-trace-state-retirement-rust"
$cliContractOutput = Join-Path $OutputDir "cli-contract"
$replContractOutput = Join-Path $OutputDir "repl-contract"
$bytecodeToolContractOutput = Join-Path $OutputDir "bytecode-tool-contract"
$traceLifecycleInput = Join-Path $projectRoot "tests/trace_fixtures"
$traceLifecycleManifest = Join-Path $projectRoot `
    "tests/compatibility/vm-trace-complete-lifecycle.json"
$traceCallbackManifest = Join-Path $projectRoot `
    "tests/compatibility/vm-trace-callback-basic.json"
$traceStateRetirementInput = Join-Path $traceLifecycleInput "coroutine_state_retirement.lua"

& $bytecodeRunner -InputPath $InputPath -CppBytecodeExe $CppBytecodeExe `
    -CppEvidenceExe $CppEvidenceExe -InfrastructureSelfTest -SelfTestTool Cpp `
    -OutputDir $bytecodeOutput
$bytecodeExit = $LASTEXITCODE
if ($bytecodeExit -ne 0) {
    throw "bytecode runner self-test failed with exit code $bytecodeExit"
}

$bytecodeDirectoryInput = Split-Path -Parent $InputPath
& $bytecodeRunner -InputPath $bytecodeDirectoryInput -CorpusMode Representative `
    -RepresentativeCount 1 -CppBytecodeExe $CppBytecodeExe `
    -CppEvidenceExe $CppEvidenceExe -InfrastructureSelfTest -SelfTestTool Cpp `
    -OutputDir $bytecodeDirectoryOutput
$bytecodeDirectoryExit = $LASTEXITCODE
if ($bytecodeDirectoryExit -ne 0) {
    throw "bytecode directory representative self-test failed with exit code $bytecodeDirectoryExit"
}

New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
$bytecodeCppNulInput = Join-Path $OutputDir "bytecode-cpp-nul-constant.lua"
[System.IO.File]::WriteAllText(
    $bytecodeCppNulInput,
    'return "\0A"',
    [System.Text.UTF8Encoding]::new($false)
)
& $bytecodeRunner -InputPath $bytecodeCppNulInput -CppBytecodeExe $CppBytecodeExe `
    -CppEvidenceExe $CppEvidenceExe -InfrastructureSelfTest -SelfTestTool Cpp `
    -OutputDir $bytecodeCppNulOutput
$bytecodeCppNulExit = $LASTEXITCODE
if ($bytecodeCppNulExit -ne 0) {
    throw "bytecode C++ NUL-constant self-test failed with exit code $bytecodeCppNulExit"
}

$bytecodeRustRecursiveInput = Join-Path $projectRoot "tests/lua/differential/value-types.lua"
& $bytecodeRunner -InputPath $bytecodeRustRecursiveInput `
    -InfrastructureSelfTest -SelfTestTool Rust -OutputDir $bytecodeRustRecursiveOutput
$bytecodeRustRecursiveExit = $LASTEXITCODE
if ($bytecodeRustRecursiveExit -ne 0) {
    throw "bytecode Rust recursive singleton self-test failed with exit code $bytecodeRustRecursiveExit"
}

$invalidBytecodeInput = Join-Path $projectRoot "tests/lua/basic/syntax_error.lua"
& $bytecodeRunner -InputPath $invalidBytecodeInput -CppBytecodeExe $CppBytecodeExe `
    -CppEvidenceExe $CppEvidenceExe -InfrastructureSelfTest -SelfTestTool Cpp `
    -OutputDir $bytecodeEmptyOutput
$bytecodeEmptyOutputExit = $LASTEXITCODE
if ($bytecodeEmptyOutputExit -eq 0) {
    throw "bytecode empty-output self-test unexpectedly succeeded"
}

& $traceRunner -InputPath $InputPath -InfrastructureSelfTest -SelfTestTool Synthetic `
    -OutputDir $traceOutput
$traceExit = $LASTEXITCODE
if ($traceExit -ne 0) {
    throw "VM trace runner self-test failed with exit code $traceExit"
}

& $traceRunner -InputPath $InputPath -CppTraceExe $CppTraceExe `
    -TraceMode Snapshot -EvidenceProfile Basic -InfrastructureSelfTest -SelfTestTool Cpp `
    -OutputDir $traceCppOutput
$traceCppExit = $LASTEXITCODE
if ($traceCppExit -ne 0) {
    throw "C++ VM snapshot producer self-test failed with exit code $traceCppExit"
}

& $traceRunner -InputPath $InputPath -RustAppExe $RustAppExe `
    -TraceMode Snapshot -EvidenceProfile Basic -InfrastructureSelfTest -SelfTestTool Rust `
    -OutputDir $traceRustOutput
$traceRustExit = $LASTEXITCODE
if ($traceRustExit -ne 0) {
    throw "Rust VM snapshot producer self-test failed with exit code $traceRustExit"
}

& $traceRunner -InputPath $InputPath -CppTraceExe $CppTraceExe `
    -TraceMode Diff -EvidenceProfile Basic -InfrastructureSelfTest -SelfTestTool Cpp `
    -OutputDir $traceCppDiffOutput
$traceCppDiffExit = $LASTEXITCODE
if ($traceCppDiffExit -ne 0) {
    throw "C++ VM diff producer self-test failed with exit code $traceCppDiffExit"
}

& $traceRunner -InputPath $InputPath -RustAppExe $RustAppExe `
    -TraceMode Diff -EvidenceProfile Basic -InfrastructureSelfTest -SelfTestTool Rust `
    -OutputDir $traceRustDiffOutput
$traceRustDiffExit = $LASTEXITCODE
if ($traceRustDiffExit -ne 0) {
    throw "Rust VM diff producer self-test failed with exit code $traceRustDiffExit"
}

& $traceRunner -InputPath $traceLifecycleInput -CppTraceExe $CppTraceExe -RustAppExe $RustAppExe `
    -CorpusMode Representative -RepresentativeCount 4 `
    -RepresentativeManifest $traceLifecycleManifest `
    -TraceMode Diff -EvidenceProfile Complete -OutputDir $traceCompleteOutput
$traceCompleteExit = $LASTEXITCODE
if ($traceCompleteExit -ne 0) {
    throw "VM Complete-profile lifecycle test failed with exit code $traceCompleteExit"
}

& $traceStateRetirementRunner -Tool Cpp -Executable $CppTraceExe `
    -InputPath $traceStateRetirementInput -OutputDir $traceStateRetirementCppOutput
$traceStateRetirementCppExit = $LASTEXITCODE
if ($traceStateRetirementCppExit -ne 0) {
    throw "C++ VM trace state-retirement test failed with exit code $traceStateRetirementCppExit"
}

& $traceStateRetirementRunner -Tool Rust -Executable $RustAppExe `
    -InputPath $traceStateRetirementInput -OutputDir $traceStateRetirementRustOutput
$traceStateRetirementRustExit = $LASTEXITCODE
if ($traceStateRetirementRustExit -ne 0) {
    throw "Rust VM trace state-retirement test failed with exit code $traceStateRetirementRustExit"
}

& $cliContractRunner -CppAppExe $CppAppExe -RustAppExe $RustAppExe `
    -OutputDir $cliContractOutput
$cliContractExit = $LASTEXITCODE
if ($cliContractExit -ne 0) {
    throw "CLI contract test failed with exit code $cliContractExit"
}

& $replContractRunner -CppAppExe $CppAppExe -RustAppExe $RustAppExe `
    -OutputDir $replContractOutput
$replContractExit = $LASTEXITCODE
if ($replContractExit -ne 0) {
    throw "REPL contract test failed with exit code $replContractExit"
}

& $bytecodeToolContractRunner -CppBytecodeExe $CppBytecodeExe `
    -RustBytecodeExe $RustBytecodeExe -OutputDir $bytecodeToolContractOutput
$bytecodeToolContractExit = $LASTEXITCODE
if ($bytecodeToolContractExit -ne 0) {
    throw "bytecode tool contract test failed with exit code $bytecodeToolContractExit"
}

& $traceRunner -InputPath $traceLifecycleInput -CppTraceExe $CppTraceExe -RustAppExe $RustAppExe `
    -CorpusMode Representative -RepresentativeCount 2 `
    -RepresentativeManifest $traceCallbackManifest `
    -TraceMode Diff -EvidenceProfile Basic -OutputDir $traceCallbackDiffOutput
$traceCallbackDiffExit = $LASTEXITCODE
if ($traceCallbackDiffExit -ne 0) {
    throw "VM callback Diff test failed with exit code $traceCallbackDiffExit"
}

& $traceRunner -InputPath $traceLifecycleInput -CppTraceExe $CppTraceExe -RustAppExe $RustAppExe `
    -CorpusMode Representative -RepresentativeCount 2 `
    -RepresentativeManifest $traceCallbackManifest `
    -TraceMode Snapshot -EvidenceProfile Basic -OutputDir $traceCallbackSnapshotOutput
$traceCallbackSnapshotExit = $LASTEXITCODE
if ($traceCallbackSnapshotExit -ne 0) {
    throw "VM callback Snapshot test failed with exit code $traceCallbackSnapshotExit"
}

$bytecodeReportPath = Join-Path $bytecodeOutput "report.json"
$bytecodeDirectoryReportPath = Join-Path $bytecodeDirectoryOutput "report.json"
$bytecodeCppNulReportPath = Join-Path $bytecodeCppNulOutput "report.json"
$bytecodeRustRecursiveReportPath = Join-Path $bytecodeRustRecursiveOutput "report.json"
$bytecodeEmptyOutputReportPath = Join-Path $bytecodeEmptyOutput "report.json"
$traceReportPath = Join-Path $traceOutput "report.json"
$traceCppReportPath = Join-Path $traceCppOutput "report.json"
$traceRustReportPath = Join-Path $traceRustOutput "report.json"
$traceCppDiffReportPath = Join-Path $traceCppDiffOutput "report.json"
$traceRustDiffReportPath = Join-Path $traceRustDiffOutput "report.json"
$traceCompleteReportPath = Join-Path $traceCompleteOutput "report.json"
$traceCallbackDiffReportPath = Join-Path $traceCallbackDiffOutput "report.json"
$traceCallbackSnapshotReportPath = Join-Path $traceCallbackSnapshotOutput "report.json"
$traceStateRetirementCppReportPath = Join-Path $traceStateRetirementCppOutput "report.json"
$traceStateRetirementRustReportPath = Join-Path $traceStateRetirementRustOutput "report.json"
$cliContractReportPath = Join-Path $cliContractOutput "report.json"
$replContractReportPath = Join-Path $replContractOutput "report.json"
$bytecodeToolContractReportPath = Join-Path $bytecodeToolContractOutput "report.json"
$bytecodeReport = Get-Content -LiteralPath $bytecodeReportPath -Raw | ConvertFrom-Json
$bytecodeDirectoryReport = Get-Content -LiteralPath $bytecodeDirectoryReportPath -Raw | ConvertFrom-Json
$bytecodeCppNulReport = Get-Content -LiteralPath $bytecodeCppNulReportPath -Raw | ConvertFrom-Json
$bytecodeRustRecursiveReport = Get-Content -LiteralPath $bytecodeRustRecursiveReportPath -Raw | ConvertFrom-Json
$bytecodeEmptyOutputReport = Get-Content -LiteralPath $bytecodeEmptyOutputReportPath -Raw | ConvertFrom-Json
$traceReport = Get-Content -LiteralPath $traceReportPath -Raw | ConvertFrom-Json
$traceCppReport = Get-Content -LiteralPath $traceCppReportPath -Raw | ConvertFrom-Json
$traceRustReport = Get-Content -LiteralPath $traceRustReportPath -Raw | ConvertFrom-Json
$traceCppDiffReport = Get-Content -LiteralPath $traceCppDiffReportPath -Raw | ConvertFrom-Json
$traceRustDiffReport = Get-Content -LiteralPath $traceRustDiffReportPath -Raw | ConvertFrom-Json
$traceCompleteReport = Get-Content -LiteralPath $traceCompleteReportPath -Raw | ConvertFrom-Json
$traceCallbackDiffReport = Get-Content -LiteralPath $traceCallbackDiffReportPath -Raw | ConvertFrom-Json
$traceCallbackSnapshotReport = Get-Content -LiteralPath $traceCallbackSnapshotReportPath -Raw | ConvertFrom-Json
$traceStateRetirementCppReport = Get-Content -LiteralPath $traceStateRetirementCppReportPath -Raw | ConvertFrom-Json
$traceStateRetirementRustReport = Get-Content -LiteralPath $traceStateRetirementRustReportPath -Raw | ConvertFrom-Json
$cliContractReport = Get-Content -LiteralPath $cliContractReportPath -Raw | ConvertFrom-Json
$replContractReport = Get-Content -LiteralPath $replContractReportPath -Raw | ConvertFrom-Json
$bytecodeToolContractReport = Get-Content -LiteralPath $bytecodeToolContractReportPath -Raw | ConvertFrom-Json

if ($bytecodeEmptyOutputReport.status -ne "infrastructure-failed" -or
    $bytecodeEmptyOutputReport.summary.selected -ne 1 -or
    $bytecodeEmptyOutputReport.summary.infrastructureFailures -lt 1 -or
    $bytecodeEmptyOutputReport.cases.Count -ne 1) {
    throw "bytecode empty-output self-test did not produce a complete fail-closed report"
}

$expectedCompleteEvidence = @(
    "eventSequence", "pcOpcode", "activeCallFrames", "stackTop",
    "changedRegisters", "upvalueLifecycle", "callReturnYieldResume", "errorValueCategory"
)
if ($traceCompleteReport.schemaVersion -ne 2 -or
    $traceCompleteReport.purpose -ne "cross-language-parity" -or
    $traceCompleteReport.traceMode -ne "Diff" -or
    $traceCompleteReport.evidenceProfile -ne "Complete" -or
    $traceCompleteReport.status -ne "passed" -or
    $traceCompleteReport.summary.selected -ne 4 -or
    $traceCompleteReport.summary.passed -ne 4 -or
    $traceCompleteReport.summary.failed -ne 0 -or
    $traceCompleteReport.summary.infrastructureFailures -ne 0 -or
    $traceCompleteReport.summary.semanticFailures -ne 0 -or
    $traceCompleteReport.cases.Count -ne 4 -or
    @(Compare-Object @($traceCompleteReport.requiredEvidence) $expectedCompleteEvidence).Count -ne 0 -or
    @($traceCompleteReport.cases | Where-Object { $_.differenceCount -ne 0 }).Count -ne 0) {
    throw "VM Complete-profile lifecycle test did not pass cleanly"
}
foreach ($completeCase in @($traceCompleteReport.cases)) {
    foreach ($evidenceName in $expectedCompleteEvidence) {
        if (-not [bool]$completeCase.evidence.left.$evidenceName -or
            -not [bool]$completeCase.evidence.right.$evidenceName) {
            throw "VM Complete-profile lifecycle case '$($completeCase.relativeInput)' lacks '$evidenceName' evidence"
        }
    }
}

foreach ($retirementReport in @($traceStateRetirementCppReport, $traceStateRetirementRustReport)) {
    if ($retirementReport.schemaVersion -ne 1 -or
        $retirementReport.purpose -ne "producer-lifecycle-self-test" -or
        $retirementReport.status -ne "passed" -or
        $retirementReport.traceMode -ne "Diff" -or
        $retirementReport.evidenceProfile -ne "Complete" -or
        $retirementReport.execution.exitCode -ne 1 -or
        $retirementReport.execution.timedOut -or
        $null -ne $retirementReport.execution.startError -or
        $retirementReport.observed.totalUpvalueOpens -ne 6 -or
        $retirementReport.observed.totalUpvalueCloses -ne 5 -or
        $retirementReport.observed.capturedUpvalueOpens -ne 3 -or
        $retirementReport.observed.capturedUpvalueCloses -ne 2 -or
        @($retirementReport.issues).Count -ne 0) {
        throw "$($retirementReport.tool) VM trace state-retirement report did not pass cleanly"
    }
    foreach ($kind in @("instruction", "call", "return", "yield", "resume", "upvalue-open", "upvalue-close", "error")) {
        if (-not (@($retirementReport.observed.eventKinds) -contains $kind)) {
            throw "$($retirementReport.tool) VM trace state-retirement report lacks '$kind'"
        }
    }
}

$expectedCliCoverage = @(
    "arguments", "stdin", "-e", "-l", "-i", "--", "script-args",
    "hash-bang", "file-errors", "source-name", "stdout-stderr-exit"
)
if ($cliContractReport.schemaVersion -ne 1 -or
    $cliContractReport.purpose -ne "cross-language-cli-contract" -or
    $cliContractReport.status -ne "passed" -or
    $cliContractReport.summary.selected -ne 18 -or
    $cliContractReport.summary.passed -ne 18 -or
    $cliContractReport.summary.failed -ne 0 -or
    $cliContractReport.summary.infrastructureFailures -ne 0 -or
    $cliContractReport.cases.Count -ne 18 -or
    @(Compare-Object @($cliContractReport.covered) $expectedCliCoverage).Count -ne 0 -or
    @($cliContractReport.cases | Where-Object { $_.status -ne "passed" }).Count -ne 0 -or
    @($cliContractReport.issues).Count -ne 0 -or
    $cliContractReport.deferred.Count -ne 1 -or
    $cliContractReport.deferred[0].id -ne "binary-chunk-execution" -or
    $cliContractReport.deferred[0].milestone -ne "M3") {
    throw "CLI contract report did not pass cleanly"
}

$expectedReplCoverage = @(
    "multiline", "incomplete-eof", "expression", "syntax-runtime-recovery",
    "state-stack-reset", "history", "meta-command", "submitted-completion",
    "prompt", "stdout-stderr-exit"
)
if ($replContractReport.schemaVersion -ne 1 -or
    $replContractReport.purpose -ne "cross-language-repl-contract" -or
    $replContractReport.status -ne "passed" -or
    $replContractReport.summary.selected -ne 8 -or
    $replContractReport.summary.passed -ne 8 -or
    $replContractReport.summary.failed -ne 0 -or
    $replContractReport.cases.Count -ne 8 -or
    @(Compare-Object @($replContractReport.covered) $expectedReplCoverage).Count -ne 0 -or
    @($replContractReport.cases | Where-Object { $_.status -ne "passed" }).Count -ne 0 -or
    @($replContractReport.issues).Count -ne 0) {
    throw "REPL contract report did not pass cleanly"
}

$expectedBytecodeToolCoverage = @(
    "arguments", "exit-status", "compact", "full", "cfg", "diff",
    "string-escaping", "nested-proto", "file-error", "compile-error",
    "json-schema", "strict-format"
)
if ($bytecodeToolContractReport.schemaVersion -ne 1 -or
    $bytecodeToolContractReport.purpose -ne "bytecode-viewer-contract" -or
    $bytecodeToolContractReport.status -ne "passed" -or
    $bytecodeToolContractReport.summary.selected -ne 15 -or
    $bytecodeToolContractReport.summary.passed -ne 15 -or
    $bytecodeToolContractReport.summary.failed -ne 0 -or
    $bytecodeToolContractReport.summary.crossLanguage -ne 12 -or
    $bytecodeToolContractReport.summary.rustExtensions -ne 3 -or
    $bytecodeToolContractReport.cases.Count -ne 15 -or
    @(Compare-Object @($bytecodeToolContractReport.covered) $expectedBytecodeToolCoverage).Count -ne 0 -or
    @($bytecodeToolContractReport.cases | Where-Object { $_.status -ne "passed" }).Count -ne 0 -or
    @($bytecodeToolContractReport.issues).Count -ne 0) {
    throw "bytecode tool contract report did not pass cleanly"
}

foreach ($callbackReport in @($traceCallbackDiffReport, $traceCallbackSnapshotReport)) {
    if ($callbackReport.schemaVersion -ne 2 -or
        $callbackReport.purpose -ne "cross-language-parity" -or
        $callbackReport.evidenceProfile -ne "Basic" -or
        $callbackReport.status -ne "passed" -or
        $callbackReport.summary.selected -ne 2 -or
        $callbackReport.summary.passed -ne 2 -or
        $callbackReport.summary.failed -ne 0 -or
        $callbackReport.summary.infrastructureFailures -ne 0 -or
        $callbackReport.summary.semanticFailures -ne 0 -or
        $callbackReport.cases.Count -ne 2 -or
        @($callbackReport.cases | Where-Object { $_.differenceCount -ne 0 }).Count -ne 0) {
        throw "VM callback $($callbackReport.traceMode) report did not pass cleanly"
    }
}
if ($traceCallbackDiffReport.traceMode -ne "Diff" -or
    $traceCallbackSnapshotReport.traceMode -ne "Snapshot") {
    throw "VM callback reports did not preserve their requested trace modes"
}
$leftCompleteKinds = @($traceCompleteReport.cases | ForEach-Object {
    @($_.evidence.left.observedEventKinds)
})
$rightCompleteKinds = @($traceCompleteReport.cases | ForEach-Object {
    @($_.evidence.right.observedEventKinds)
})
foreach ($kind in @("instruction", "call", "return", "yield", "resume", "upvalue-open", "upvalue-close", "error")) {
    if (-not ($leftCompleteKinds -contains $kind) -or
        -not ($rightCompleteKinds -contains $kind)) {
        throw "VM Complete-profile lifecycle test did not observe '$kind' on both sides"
    }
}
foreach ($completeCase in @($traceCompleteReport.cases)) {
    foreach ($execution in @($completeCase.executions.left, $completeCase.executions.right)) {
        if ($execution.timedOut -or $null -ne $execution.startError -or $execution.exitCode -ne 1) {
            throw "VM Complete-profile lifecycle case '$($completeCase.relativeInput)' lacks the expected matched exit-1 process evidence"
        }
    }
}

foreach ($item in @(
    [pscustomobject]@{ name = "bytecode"; report = $bytecodeReport },
    [pscustomobject]@{ name = "bytecode-directory-representative"; report = $bytecodeDirectoryReport },
    [pscustomobject]@{ name = "bytecode-cpp-nul-constant"; report = $bytecodeCppNulReport },
    [pscustomobject]@{ name = "bytecode-rust-recursive-singleton"; report = $bytecodeRustRecursiveReport },
    [pscustomobject]@{ name = "vm-trace"; report = $traceReport },
    [pscustomobject]@{ name = "vm-trace-cpp-snapshot"; report = $traceCppReport },
    [pscustomobject]@{ name = "vm-trace-rust-snapshot"; report = $traceRustReport },
    [pscustomobject]@{ name = "vm-trace-cpp-diff"; report = $traceCppDiffReport },
    [pscustomobject]@{ name = "vm-trace-rust-diff"; report = $traceRustDiffReport }
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

$cppNulEvidence = $bytecodeCppNulReport.cases[0].executions.leftEvidence.stdout |
    ConvertFrom-Json
if ($cppNulEvidence.schema_version -ne 2 -or
    @($cppNulEvidence.protos).Count -ne 1 -or
    @($cppNulEvidence.protos[0].constants).Count -ne 1 -or
    $cppNulEvidence.protos[0].constants[0].type -ne "string" -or
    $cppNulEvidence.protos[0].constants[0].value.bytes -cne "0041") {
    throw "bytecode C++ NUL-constant self-test lost byte-safe constant evidence"
}

$rustRecursiveJson = $bytecodeRustRecursiveReport.cases[0].executions.left.stdout |
    ConvertFrom-Json
if (@($rustRecursiveJson.sub_protos).Count -ne 1 -or
    @($rustRecursiveJson.sub_protos[0].line_info).Count -ne 1) {
    throw "bytecode Rust recursive singleton self-test fixture lost its one-entry child line_info"
}

foreach ($report in @($bytecodeReport, $bytecodeDirectoryReport, $bytecodeCppNulReport)) {
    foreach ($case in @($report.cases)) {
        foreach ($execution in @(
            $case.executions.leftEvidence,
            $case.executions.rightEvidence
        )) {
            if ($null -eq $execution -or
                $null -eq $execution.command -or
                $null -eq $execution.stdoutFile -or
                $null -eq $execution.stderrFile -or
                $execution.exitCode -ne 0 -or
                $execution.timedOut -or
                $null -ne $execution.startError) {
                throw "bytecode self-test is missing clean C++ byte-safe process evidence"
            }
        }
        if (-not [bool]$case.evidence.left.localNames -or
            -not [bool]$case.evidence.right.localNames -or
            -not [bool]$case.evidence.left.constantValues -or
            -not [bool]$case.evidence.right.constantValues) {
            throw "bytecode self-test did not merge C++ byte-safe evidence"
        }
    }
}

foreach ($evidence in $traceReport.requiredEvidence) {
    if (-not [bool]$traceReport.cases[0].evidence.left.$evidence) {
        throw "VM trace self-test fixture did not cover required evidence '$evidence'"
    }
}

foreach ($producer in @($traceCppReport, $traceRustReport, $traceCppDiffReport, $traceRustDiffReport)) {
    if ($producer.traceMode -notin @("Snapshot", "Diff") -or $producer.evidenceProfile -ne "Basic") {
        throw "real VM trace self-test did not use a supported basic profile"
    }
    foreach ($evidence in $producer.requiredEvidence) {
        if (-not [bool]$producer.cases[0].evidence.left.$evidence -or
            -not [bool]$producer.cases[0].evidence.right.$evidence) {
            throw "real VM trace producer did not cover required evidence '$evidence'"
        }
    }
}

$summary = [ordered]@{
    schemaVersion = 1
    status = "passed"
    bytecodeReport = $bytecodeReportPath
    bytecodeDirectoryReport = $bytecodeDirectoryReportPath
    bytecodeCppNulReport = $bytecodeCppNulReportPath
    bytecodeRustRecursiveReport = $bytecodeRustRecursiveReportPath
    bytecodeEmptyOutputReport = $bytecodeEmptyOutputReportPath
    vmTraceReport = $traceReportPath
    vmTraceCppSnapshotReport = $traceCppReportPath
    vmTraceRustSnapshotReport = $traceRustReportPath
    vmTraceCppDiffReport = $traceCppDiffReportPath
    vmTraceRustDiffReport = $traceRustDiffReportPath
    vmTraceCompleteReport = $traceCompleteReportPath
    vmTraceCallbackDiffReport = $traceCallbackDiffReportPath
    vmTraceCallbackSnapshotReport = $traceCallbackSnapshotReportPath
    vmTraceStateRetirementCppReport = $traceStateRetirementCppReportPath
    vmTraceStateRetirementRustReport = $traceStateRetirementRustReportPath
    cliContractReport = $cliContractReportPath
    replContractReport = $replContractReportPath
    bytecodeToolContractReport = $bytecodeToolContractReportPath
}
$summary | ConvertTo-Json -Depth 8
exit 0
