param(
    [string]$Root = "",
    [string]$InventoryPath = "tests/compatibility/string_access_inventory.json",
    [string]$ResultPath = "target/compatibility/string-contract.json"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
} else {
    $Root = (Resolve-Path -LiteralPath $Root).Path
}

function Resolve-RootedPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $Root $Path))
}

function Get-ProductionPrefix {
    param([Parameter(Mandatory = $true)][string]$Path)
    $lines = Get-Content -LiteralPath $Path
    $result = [System.Collections.Generic.List[string]]::new()
    foreach ($line in $lines) {
        if ($line -match '^\s*#\[cfg\(test\)\]') {
            break
        }
        $result.Add($line)
    }
    return ($result -join "`n")
}

$inventoryFile = Resolve-RootedPath $InventoryPath
if (-not (Test-Path -LiteralPath $inventoryFile -PathType Leaf)) {
    throw "String access inventory is missing: $inventoryFile"
}
$inventory = Get-Content -LiteralPath $inventoryFile -Raw | ConvertFrom-Json
if ($inventory.schemaVersion -ne 1) {
    throw "String access inventory must use schemaVersion 1"
}

$violations = [System.Collections.Generic.List[string]]::new()
$checkedFiles = [System.Collections.Generic.HashSet[string]]::new(
    [System.StringComparer]::OrdinalIgnoreCase
)

foreach ($relative in @($inventory.productionReadPaths)) {
    $absolute = Resolve-RootedPath ([string]$relative)
    if (-not (Test-Path -LiteralPath $absolute -PathType Leaf)) {
        $violations.Add("inventory path is missing: $relative")
        continue
    }
    $null = $checkedFiles.Add([string]$relative)
    $source = Get-ProductionPrefix $absolute
    $unsafeStringPatterns = @(
        'unsafe\s*\{\s*(?:s|string(?:_ref)?|source_ref|name_ref|key_ref|message_ref|mask_ref|mode_ref|template_ref|option_ref|path_ref|value_ref|candidate)\.as_ref\(\)',
        'unsafe\s*\{[^}`r`n]*\.as_ref\(\)\s*\}\s*\.as_bytes\(\)',
        'map\s*\(\s*GcString::as_bytes\s*\)'
    )
    foreach ($pattern in $unsafeStringPatterns) {
        if ([regex]::IsMatch($source, $pattern)) {
            $violations.Add("$relative contains unscoped GcString content access ($pattern)")
        }
    }
}

$crateFiles = Get-ChildItem -LiteralPath (Join-Path $Root "crates") `
    -Recurse -File -Filter "*.rs"
$rawConstructorAllowlist = @(
    "crates/lua_core/src/string_pool.rs",
    "crates/lua_core/src/gc/publication.rs"
)
foreach ($file in $crateFiles) {
    $relative = $file.FullName.Substring($Root.Length).TrimStart("\", "/").Replace("\", "/")
    $source = Get-ProductionPrefix $file.FullName
    if ($source -match 'GcString::from_(?:bytes|utf8_text)\s*\(' -and
        $rawConstructorAllowlist -notcontains $relative) {
        $violations.Add("$relative bypasses the canonical StringPool constructor boundary")
    }
}

$compilerSource = Get-Content -LiteralPath (
    Join-Path $Root "crates/lua_compiler/src/codegen/mod.rs"
) -Raw
foreach ($pattern in @(
    "impl<'services>\s+CodeGenerator<'services,\s*GarbageCollector>\s*\{[\s\S]*?pub\s+fn\s+new\s*\(",
    "impl<'services,\s*'scope>\s+CodeGenerator<'services,\s*PublicationTxn<'scope>>\s*\{[\s\S]*?pub\s+fn\s+new_in_publication\s*\(",
    'string_pool\s*:\s*Option\s*<',
    'None\s*=>\s*(?:self\.)?(?:create|alloc)\s*\(\s*GcString'
)) {
    if ($compilerSource -match $pattern) {
        $violations.Add("compiler permits non-canonical strings ($pattern)")
    }
}

$valueSource = Get-Content -LiteralPath (
    Join-Path $Root "crates/lua_core/src/value.rs"
) -Raw
if ($valueSource -notmatch 'Value::String\(a\),\s*Value::String\(b\)\)\s*=>\s*a\s*==\s*b') {
    $violations.Add("Value::String equality is not canonical GcRef identity")
}
if ($valueSource -notmatch 'Value::String\(s\)\s*=>\s*s\.hash\(state\)') {
    $violations.Add("Value::String hashing is not canonical GcRef identity")
}
if ($valueSource -match 'Value::String[\s\S]{0,240}?unsafe\s*\{[^}]*\.as_ref\(\)') {
    $violations.Add("Value::String Eq/Hash path dereferences managed string memory")
}

$result = [ordered]@{
    schemaVersion = 1
    channel = "string-contract"
    passed = $violations.Count -eq 0
    inventory = $inventoryFile.Substring($Root.Length).TrimStart("\", "/").Replace("\", "/")
    checkedProductionPaths = @($checkedFiles) | Sort-Object
    violations = @($violations)
}

$resultFile = Resolve-RootedPath $ResultPath
$resultDirectory = Split-Path -Parent $resultFile
if (-not [string]::IsNullOrWhiteSpace($resultDirectory)) {
    New-Item -ItemType Directory -Force -Path $resultDirectory | Out-Null
}
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $resultFile -Encoding utf8

if ($violations.Count -ne 0) {
    @($violations) | ForEach-Object { Write-Error $_ }
    exit 1
}

Write-Output "String contract passed for $($checkedFiles.Count) production paths."
