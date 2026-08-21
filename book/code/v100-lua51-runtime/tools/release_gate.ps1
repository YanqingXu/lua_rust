param(
    [switch]$RunOracles,
    [string]$OfficialLua = "",
    [string]$CppLua = "",
    [string]$CppRoot = "",
    [string]$CppArchive = ""
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$snapshotRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$manifest = Join-Path $snapshotRoot 'Cargo.toml'
$hostLine = (& rustc -vV | Select-String -Pattern '^host:').Line
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($hostLine)) {
    throw 'Could not determine the native Rust host triple'
}
$hostTriple = ($hostLine -split ':', 2)[1].Trim()

function Assert-LastExitCode([string]$Step) {
    if ($LASTEXITCODE -ne 0) {
        throw "$Step failed with exit code $LASTEXITCODE"
    }
}

Write-Host '==> rustfmt'
& cargo fmt --manifest-path $manifest --all -- --check
Assert-LastExitCode 'rustfmt'

Write-Host '==> clippy'
& cargo clippy --manifest-path $manifest --workspace --all-targets `
    --target $hostTriple -- -D warnings
Assert-LastExitCode 'clippy'

Write-Host '==> debug tests'
& cargo test --manifest-path $manifest --workspace --target $hostTriple
Assert-LastExitCode 'debug tests'

Write-Host '==> release tests'
& cargo test --manifest-path $manifest --workspace --release --target $hostTriple
Assert-LastExitCode 'release tests'

Write-Host '==> release example contract'
$example = Join-Path $snapshotRoot 'examples/demo.lua'
$exampleOutput = (& cargo run --quiet --manifest-path $manifest --target $hostTriple -- $example |
    Out-String).Replace("`r`n", "`n")
Assert-LastExitCode 'release example'
if ($exampleOutput -cne "100`n") {
    throw "Unexpected example stdout: $($exampleOutput | ConvertTo-Json -Compress)"
}

Write-Host '==> official opcode audit contract'
$bytecodeManifest = Join-Path $snapshotRoot 'crates/lua_bytecode/Cargo.toml'
$audit = (& cargo run --quiet --manifest-path $bytecodeManifest --target $hostTriple -- `
    --format opcode-audit | Out-String) |
    ConvertFrom-Json
Assert-LastExitCode 'opcode audit'
if ($audit.schema -cne 'lua-rust-lua51-opcode-audit/v1' -or
    $audit.binary_chunk_compatible -ne $false -or
    @($audit.opcodes).Count -ne 38) {
    throw 'Opcode audit contract is incomplete or overclaims binary compatibility'
}

if ($RunOracles) {
    foreach ($required in @{
        OfficialLua = $OfficialLua
        CppLua = $CppLua
        CppRoot = $CppRoot
    }.GetEnumerator()) {
        if ([string]::IsNullOrWhiteSpace([string]$required.Value)) {
            throw "-RunOracles requires -$($required.Key)"
        }
    }
    Write-Host '==> pinned dual-oracle differential'
    $arguments = @{
        OfficialLua = $OfficialLua
        CppLua = $CppLua
        CppRoot = $CppRoot
    }
    if (-not [string]::IsNullOrWhiteSpace($CppArchive)) {
        $arguments.CppArchive = $CppArchive
    }
    & (Join-Path $PSScriptRoot 'run_differential.ps1') @arguments
    Assert-LastExitCode 'dual-oracle differential'
} else {
    Write-Host '==> pinned dual-oracle differential skipped (pass -RunOracles with oracle paths)'
}

Write-Host 'v1.0 release gate passed.'
