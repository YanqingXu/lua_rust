param(
    [string]$Root = "",
    [string]$InventoryPath = "tests/compatibility/gc_mutation_inventory.json",
    [string]$ResultPath = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
} else {
    $Root = (Resolve-Path -LiteralPath $Root).Path
}

$failures = [System.Collections.Generic.List[string]]::new()
$knownFamilies = @(
    "TABLE_EDGES",
    "FUNCTION_EDGES",
    "PROTO_EDGES",
    "UPVALUE_EDGES",
    "USERDATA_EDGES",
    "THREAD_EDGES",
    "STATE_ROOTS",
    "RUNTIME_ROOTS"
)
$knownStatuses = @("implemented", "construction_only", "atomic_rescan")
$resolvedInventory = [System.IO.Path]::GetFullPath((Join-Path $Root $InventoryPath))
$families = @()

try {
    if (-not (Test-Path -LiteralPath $resolvedInventory -PathType Leaf)) {
        throw "Inventory file does not exist: $resolvedInventory"
    }
    $inventory = Get-Content -LiteralPath $resolvedInventory -Raw | ConvertFrom-Json
    if ($inventory.schema_version -ne 1) {
        $failures.Add("gc mutation inventory schema_version must be 1")
    }
    if ($inventory.mutation_context.symbol -ne "GarbageCollector::with_mut" -or
        $inventory.mutation_context.barrier_symbol -ne
            "GarbageCollector::after_managed_mutation") {
        $failures.Add("inventory must name the checked mutation context and barrier")
    }
    if ($inventory.mutation_context.automatic_trigger -ne "disabled") {
        $failures.Add("allocation-triggered collection must remain disabled in M1.11")
    }

    $families = @($inventory.families)
    $seen = @{}
    foreach ($entry in $families) {
        $family = [string]$entry.family
        if ($knownFamilies -notcontains $family) {
            $failures.Add("unknown mutation family '$family'")
        } elseif ($seen.ContainsKey($family)) {
            $failures.Add("duplicate mutation family '$family'")
        } else {
            $seen[$family] = $true
        }
        if ($knownStatuses -notcontains [string]$entry.status) {
            $failures.Add("mutation family '$family' has invalid status")
        }
        if ([string]::IsNullOrWhiteSpace([string]$entry.mechanism)) {
            $failures.Add("mutation family '$family' requires a mechanism")
        }
        if (@($entry.owner_symbols).Count -eq 0 -or
            @($entry.production_locations).Count -eq 0) {
            $failures.Add("mutation family '$family' requires symbols and locations")
        }
        foreach ($location in @($entry.production_locations)) {
            $path = Join-Path $Root ([string]$location.path)
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
                $failures.Add(
                    "mutation family '$family' source is missing: $($location.path)"
                )
                continue
            }
            $source = Get-Content -LiteralPath $path -Raw
            $tokens = @(
                ([string]$location.symbol) -split '[^A-Za-z0-9_]+' |
                    Where-Object { $_.Length -ge 4 }
            )
            if ($tokens.Count -ne 0 -and
                -not ($tokens | Where-Object { $source.Contains($_) })) {
                $failures.Add(
                    "mutation family '$family' symbol anchor is stale: " +
                    "$($location.path) :: $($location.symbol)"
                )
            }
        }
    }
    foreach ($family in $knownFamilies) {
        if (-not $seen.ContainsKey($family)) {
            $failures.Add("inventory is missing mutation family '$family'")
        }
    }

    $requiredSources = @{
        "crates/lua_core/src/gc/collector.rs" = @(
            "after_managed_mutation",
            "managed_state_edge"
        )
        "crates/lua_core/src/gc/incremental.rs" = @(
            "IncrementalPhase::Propagate",
            "IncrementalPhase::Atomic",
            "IncrementalPhase::Sweep",
            "IncrementalPhase::Finalize",
            "publish_new_allocation"
        )
        "crates/lua_vm/src/runtime/root_trace.rs" = @(
            "IncrementalRootTrace",
            "atomic_rescan",
            "seed_runtime_snapshot"
        )
        "crates/lua_vm/src/runtime/incremental_collection.rs" = @(
            "collect_incremental_step_at_safe_point",
            "incremental_sweep_step"
        )
    }
    foreach ($relative in $requiredSources.Keys) {
        $source = Get-Content -LiteralPath (Join-Path $Root $relative) -Raw
        foreach ($anchor in $requiredSources[$relative]) {
            if (-not $source.Contains($anchor)) {
                $failures.Add("$relative lacks required mutation/phase anchor '$anchor'")
            }
        }
    }

    $productionFiles = @(
        Get-ChildItem -LiteralPath (Join-Path $Root "crates") -Recurse -Filter "*.rs" |
            Where-Object {
                $_.FullName -notmatch '[\\/]tests[\\/]' -and
                $_.FullName -notmatch '[\\/]target[\\/]'
            }
    )
    $rawEdgePattern = [regex]::new(
        'as\s+\*mut\s+(Table|Function|Upvalue|Thread)\b.*\.(set|set_array|set_metatable|set_env|set_upvalue|add_upvalue|set_closed_value|close|set_caller|set_state_handle)\s*\('
    )
    $namedEdgeSetterPattern = [regex]::new(
        '\.(set_array|set_metatable|set_env|set_upvalue|add_upvalue|set_closed_value|set_caller|set_state_handle)\s*\(|\bupvalue\w*\s*\.\s*close\s*\('
    )
    $tableSetPattern = [regex]::new(
        '\b(table|global|metatable|state)\w*\s*\.\s*set\s*\('
    )
    $contextExempt = @(
        "crates\lua_core\src\table.rs",
        "crates\lua_core\src\function.rs",
        "crates\lua_core\src\proto.rs",
        "crates\lua_core\src\upvalue.rs",
        "crates\lua_core\src\userdata.rs",
        "crates\lua_core\src\thread.rs",
        "crates\lua_core\src\gc\publication.rs",
        "crates\lua_core\src\gc\weak.rs"
    )
    foreach ($file in $productionFiles) {
        $relative = [System.IO.Path]::GetRelativePath($Root, $file.FullName)
        $lines = @(Get-Content -LiteralPath $file.FullName)
        for ($index = 0; $index -lt $lines.Count; $index++) {
            if ($lines[$index] -match '^\s*#\[cfg\(test\)\]') {
                break
            }
            if ($rawEdgePattern.IsMatch($lines[$index])) {
                $failures.Add(
                    "raw managed edge mutation bypass: ${relative}:$($index + 1)"
                )
            }
            if (($namedEdgeSetterPattern.IsMatch($lines[$index]) -or
                $tableSetPattern.IsMatch($lines[$index])) -and
                $contextExempt -notcontains $relative -and
                $relative -notlike "crates\lua_compiler\src\*") {
                $contextStart = [Math]::Max(0, $index - 10)
                $context = ($lines[$contextStart..$index] -join "`n")
                if ($context -notmatch '\b(with_mut|transaction\.with_mut)\s*\(') {
                    $failures.Add(
                        "managed edge setter lacks checked mutation context: " +
                        "${relative}:$($index + 1):$($lines[$index].Trim())"
                    )
                }
            }
        }
    }

    $allProduction = ($productionFiles | ForEach-Object {
        $lines = @(Get-Content -LiteralPath $_.FullName)
        $cut = $lines.Count
        for ($i = 0; $i -lt $lines.Count; $i++) {
            if ($lines[$i] -match '^\s*#\[cfg\(test\)\]') {
                $cut = $i
                break
            }
        }
        ($lines[0..([Math]::Max(0, $cut - 1))] -join "`n")
    }) -join "`n"
    foreach ($legacy in @("gc_step_remaining", "gc_stopped", "collection_step_completed")) {
        if ($allProduction.Contains($legacy)) {
            $failures.Add("legacy fake incremental control remains: $legacy")
        }
    }
} catch {
    $failures.Add("validator failed closed: $($_.Exception.Message)")
}

$result = [ordered]@{
    schema_version = 1
    check = "gc-mutation-contract"
    inventory_path = $resolvedInventory
    expected_families = $knownFamilies.Count
    checked_families = if ($null -ne $families) { $families.Count } else { 0 }
    valid = $failures.Count -eq 0
    failures = @($failures)
}
$json = $result | ConvertTo-Json -Depth 8
if (-not [string]::IsNullOrWhiteSpace($ResultPath)) {
    $resolvedResult = if ([System.IO.Path]::IsPathRooted($ResultPath)) {
        [System.IO.Path]::GetFullPath($ResultPath)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $Root $ResultPath))
    }
    [System.IO.Directory]::CreateDirectory((Split-Path -Parent $resolvedResult)) |
        Out-Null
    Set-Content -LiteralPath $resolvedResult -Value $json -Encoding utf8
}
Write-Output $json
if ($failures.Count -ne 0) {
    exit 1
}
exit 0
