$ErrorActionPreference = 'Stop'

$hostLine = (& rustc -vV | Select-String -Pattern '^host:').Line
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($hostLine)) {
    throw 'Could not determine the native Rust host triple'
}
$hostTriple = ($hostLine -split ':', 2)[1].Trim()

$snapshotNames = @(
    'v00-skeleton',
    'v01-value',
    'v02-lexer',
    'v03-parser',
    'v04-expressions',
    'v05-bytecode-vm',
    'v06-variables-scope',
    'v07-control-flow',
    'v08-table',
    'v09-functions',
    'v10-multi-return-vararg',
    'v11-closure-upvalue',
    'v12-metatable',
    'v13-runtime-gc',
    'v14-incremental-gc',
    'v15-coroutine',
    'v16-core-stdlib',
    'v17-host-stdlib-modules',
    'v18-cli-tools'
    'v100-lua51-runtime'
)

$codeRoot = $PSScriptRoot

foreach ($snapshotName in $snapshotNames) {
    $manifestPath = Join-Path $codeRoot "$snapshotName/Cargo.toml"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "Missing teaching snapshot manifest: $manifestPath"
    }

    Write-Host "==> Checking $snapshotName"

    & cargo fmt --manifest-path $manifestPath --all -- --check
    if ($LASTEXITCODE -ne 0) {
        throw "cargo fmt failed for $snapshotName"
    }

    & cargo clippy --manifest-path $manifestPath --workspace --all-targets `
        --target $hostTriple -- -D warnings
    if ($LASTEXITCODE -ne 0) {
        throw "cargo clippy failed for $snapshotName"
    }

    & cargo test --manifest-path $manifestPath --workspace --target $hostTriple
    if ($LASTEXITCODE -ne 0) {
        throw "cargo test failed for $snapshotName"
    }

    if ($snapshotName -eq 'v100-lua51-runtime') {
        & cargo test --manifest-path $manifestPath --workspace --release `
            --target $hostTriple
        if ($LASTEXITCODE -ne 0) {
            throw "cargo test --release failed for $snapshotName"
        }
    }
}

Write-Host "All $($snapshotNames.Count) teaching snapshots passed."
