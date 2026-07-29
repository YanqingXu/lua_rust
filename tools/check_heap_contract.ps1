param(
    [string]$Root = "",
    [string]$ResultPath = "target/compatibility/heap-contract.json"
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

$violations = [System.Collections.Generic.List[string]]::new()
$checkedFiles = [System.Collections.Generic.HashSet[string]]::new(
    [System.StringComparer]::OrdinalIgnoreCase
)

function Read-RequiredSource {
    param([Parameter(Mandatory = $true)][string]$RelativePath)
    $absolute = Resolve-RootedPath $RelativePath
    if (-not (Test-Path -LiteralPath $absolute -PathType Leaf)) {
        $violations.Add("required ownership source is missing: $RelativePath")
        return ""
    }
    $null = $checkedFiles.Add($RelativePath.Replace("\", "/"))
    return Get-Content -LiteralPath $absolute -Raw
}

function Assert-Contains {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Pattern
    )
    if ($Source -notmatch $Pattern) {
        $violations.Add("$Name is missing required ownership invariant ($Pattern)")
    }
}

$heapSource = Read-RequiredSource "crates/lua_core/src/heap.rs"
Assert-Contains "Heap" $heapSource 'pub\s+struct\s+Heap\s*\{'
Assert-Contains "Heap" $heapSource 'collector\s*:\s*GarbageCollector'
Assert-Contains "Heap" $heapSource 'strings\s*:\s*StringPool'
Assert-Contains "Heap" $heapSource 'impl\s+Drop\s+for\s+Heap'
Assert-Contains "Heap" $heapSource 'pub\s+fn\s+destroy_all\s*\('

$collectorSource = Read-RequiredSource "crates/lua_core/src/gc/collector.rs"
Assert-Contains "GarbageCollector" $collectorSource 'heap_id\s*:\s*HeapId'
Assert-Contains "GarbageCollector" $collectorSource `
    'destroy_object_without_pool\s*\('
Assert-Contains "GarbageCollector" $collectorSource `
    'impl\s+Drop\s+for\s+GarbageCollector'

$markSource = Read-RequiredSource "crates/lua_core/src/gc/mark.rs"
Assert-Contains "mark-only root seed" $markSource `
    'pending_finalizers_seeded\s*:\s*usize'
Assert-Contains "mark-only root seed" $markSource `
    'self\.pending_finalizers\.clone\s*\(\s*\)'

$poolSource = Read-RequiredSource "crates/lua_core/src/string_pool.rs"
Assert-Contains "StringPool" $poolSource 'heap_id\s*:\s*Option\s*<\s*HeapId\s*>'
Assert-Contains "StringPool" $poolSource 'bind_or_assert_owner\s*\('

$stateSource = Read-RequiredSource "crates/lua_vm/src/state/lua_state.rs"
foreach ($pattern in @(
    '\bpub\s+(?:gc|string_pool)\s*:',
    '\b(?:gc|string_pool)\s*:\s*Option\s*<\s*\*mut\s+(?:GarbageCollector|StringPool)'
)) {
    if ($stateSource -match $pattern) {
        $violations.Add("LuaState retains a GC/StringPool backpointer ($pattern)")
    }
}

$contextSource = Read-RequiredSource "crates/lua_vm/src/state/vm_context.rs"
Assert-Contains "VM context" $contextSource 'thread_local!\s*\{'
Assert-Contains "VM context" $contextSource 'struct\s+ActiveVmContext'
Assert-Contains "VM context" $contextSource `
    'impl\s+Drop\s+for\s+ActiveVmContext'
Assert-Contains "VM context" $contextSource 'bind_or_assert_owner\s*\('

$runtimeSource = Read-RequiredSource "crates/lua_vm/src/runtime.rs"
Assert-Contains "Runtime storage" $runtimeSource 'struct\s+RuntimeStorage\s*\{'
Assert-Contains "Runtime storage" $runtimeSource '\bheap\s*:\s*Heap'
Assert-Contains "Runtime fixed roots" $runtimeSource `
    '\bfixed_strings\s*:\s*Vec\s*<\s*GcRef\s*<\s*GcString\s*>\s*>'
if ($runtimeSource -match '\bRuntimeHeap\b') {
    $violations.Add("the superseded RuntimeHeap wrapper name has returned")
}

$rootTraceSource = Read-RequiredSource "crates/lua_vm/src/runtime/root_trace.rs"
Assert-Contains "Runtime root tracer" $rootTraceSource `
    'pub\s+fn\s+trace_roots_mark_only\s*\('
Assert-Contains "Runtime root tracer" $rootTraceSource `
    'RuntimeRootKind::PendingFinalizers'
Assert-Contains "Runtime root tracer" $rootTraceSource `
    'RuntimeRootKind::TemporaryProtectedRoots'
Assert-Contains "Runtime root tracer" $rootTraceSource `
    'RuntimeRootKind::FixedStrings'

$fullCollectionSource =
    Read-RequiredSource "crates/lua_vm/src/runtime/full_collection.rs"
Assert-Contains "Runtime full collection" $fullCollectionSource `
    'pub\(crate\)\s+fn\s+collect_full_stw\s*\('
Assert-Contains "Runtime full collection" $fullCollectionSource `
    'trace_roots_mark_only_at_safe_point\s*\('
Assert-Contains "Runtime full collection" $fullCollectionSource `
    'sweep_unreachable_owned\s*\('
Assert-Contains "Runtime full collection" $fullCollectionSource `
    'gc\.sweep\s*\(\s*strings\s*\)'
Assert-Contains "Runtime full collection" $fullCollectionSource `
    'prepare_finalizable_userdata\s*\(\s*\)'
Assert-Contains "Runtime full collection" $fullCollectionSource `
    'clear_weak_table_entries\s*\(\s*\)'
if ($fullCollectionSource -match 'pub\s+fn\s+collect_full_stw\s*\(') {
    $violations.Add(
        "Runtime full collection bypasses the sealed Runtime-native safe-point route"
    )
}

$baseSource = Get-ProductionPrefix (
    Resolve-RootedPath "crates/lua_stdlib/src/base.rs"
)
if ($baseSource -match '\bcollect_full_stw(?:_at_safe_point)?\s*\(') {
    $violations.Add(
        "Lua-visible collectgarbage bypasses the sealed Runtime-native request"
    )
}
Assert-Contains "Lua-visible collectgarbage" $baseSource `
    'RuntimeNativeFunction::CollectGarbage'
Assert-Contains "Runtime full collection scheduler" $runtimeSource `
    'RuntimeRequest::FullCollection'
Assert-Contains "Runtime finalizer drain" $runtimeSource `
    'begin_finalizer_drain\s*\(\s*\)'

$productionRoots = @(
    "crates/lua_app/src",
    "crates/lua_bytecode/src",
    "crates/lua_compiler/src",
    "crates/lua_stdlib/src",
    "crates/lua_vm/src"
)
$productionFiles = foreach ($relativeRoot in $productionRoots) {
    Get-ChildItem -LiteralPath (Resolve-RootedPath $relativeRoot) `
        -Recurse -File -Filter "*.rs"
}
foreach ($file in $productionFiles) {
    $relative = $file.FullName.Substring($Root.Length).TrimStart("\", "/").
        Replace("\", "/")
    $null = $checkedFiles.Add($relative)
    $source = Get-ProductionPrefix $file.FullName
    foreach ($pattern in @(
        'GarbageCollector::new\s*\(',
        'StringPool::new\s*\('
    )) {
        if ($source -match $pattern) {
            $violations.Add(
                "$relative constructs standalone production heap services ($pattern)"
            )
        }
    }
}

$result = [ordered]@{
    schemaVersion = 1
    channel = "heap-contract"
    passed = $violations.Count -eq 0
    checkedProductionPaths = @($checkedFiles) | Sort-Object
    violations = @($violations)
}

$resultFile = Resolve-RootedPath $ResultPath
$resultDirectory = Split-Path -Parent $resultFile
if (-not [string]::IsNullOrWhiteSpace($resultDirectory)) {
    New-Item -ItemType Directory -Force -Path $resultDirectory | Out-Null
}
$result | ConvertTo-Json -Depth 8 |
    Set-Content -LiteralPath $resultFile -Encoding utf8

if ($violations.Count -ne 0) {
    @($violations) | ForEach-Object { Write-Error $_ }
    exit 1
}

Write-Output "Heap contract passed for $($checkedFiles.Count) production paths."
