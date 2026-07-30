param(
    [string]$Root = "",
    [string]$InventoryPath = "tests/compatibility/allocator_accounting_inventory.json",
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
    "GC_OBJECTS",
    "DYNAMIC_OBJECT_CONTAINERS",
    "STRING_POOL_KEYS",
    "STATE_ARENA",
    "SHUTDOWN_ZERO",
    "AUTOMATIC_SAFE_POINT"
)
$resolvedInventory = [System.IO.Path]::GetFullPath((Join-Path $Root $InventoryPath))
$families = @()

try {
    if (-not (Test-Path -LiteralPath $resolvedInventory -PathType Leaf)) {
        throw "Inventory file does not exist: $resolvedInventory"
    }
    $inventory = Get-Content -LiteralPath $resolvedInventory -Raw | ConvertFrom-Json
    if ($inventory.schema_version -ne 1) {
        $failures.Add("allocator inventory schema_version must be 1")
    }
    if ($inventory.contract.metric -ne "managed_payload_bytes") {
        $failures.Add("allocator inventory must name the managed payload metric")
    }

    $families = @($inventory.families)
    $seen = @{}
    foreach ($entry in $families) {
        $family = [string]$entry.family
        if ($knownFamilies -notcontains $family) {
            $failures.Add("unknown allocator family '$family'")
        } elseif ($seen.ContainsKey($family)) {
            $failures.Add("duplicate allocator family '$family'")
        } else {
            $seen[$family] = $true
        }
        if ([string]$entry.status -ne "implemented") {
            $failures.Add("allocator family '$family' is not implemented")
        }
        $sourcePath = Join-Path $Root ([string]$entry.path)
        if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
            $failures.Add("allocator family '$family' source is missing: $($entry.path)")
            continue
        }
        $source = Get-Content -LiteralPath $sourcePath -Raw
        foreach ($anchor in @($entry.anchors)) {
            if (-not $source.Contains([string]$anchor)) {
                $failures.Add(
                    "allocator family '$family' lacks anchor '$anchor' in $($entry.path)"
                )
            }
        }
    }
    foreach ($family in $knownFamilies) {
        if (-not $seen.ContainsKey($family)) {
            $failures.Add("allocator inventory is missing family '$family'")
        }
    }

    $allocatorSource =
        Get-Content -LiteralPath (Join-Path $Root "crates/lua_core/src/allocator.rs") -Raw
    foreach ($anchor in @(
        "pub live_bytes: usize",
        "pub peak_bytes: usize",
        "pub total_allocated_bytes: usize",
        "replace_component",
        "impl Drop for AllocationAccount"
    )) {
        if (-not $allocatorSource.Contains($anchor)) {
            $failures.Add("allocator ledger lacks required anchor '$anchor'")
        }
    }

    $sweepSource =
        Get-Content -LiteralPath (Join-Path $Root "crates/lua_core/src/gc/sweep.rs") -Raw
    foreach ($anchor in @("live.accounted_size", "live.allocator_size")) {
        if (-not $sweepSource.Contains($anchor)) {
            $failures.Add("sweep must subtract stored side-table size '$anchor'")
        }
    }
    if ($sweepSource.Contains("object_size_of")) {
        $failures.Add("sweep must not recompute allocation size through object_size_of")
    }

    $executeSource =
        Get-Content -LiteralPath (Join-Path $Root "crates/lua_vm/src/execute.rs") -Raw
    foreach ($anchor in @("automatic_step_due", "VmExit::AutomaticGc")) {
        if (-not $executeSource.Contains($anchor)) {
            $failures.Add("VM automatic checkpoint lacks '$anchor'")
        }
    }
} catch {
    $failures.Add("validator failed closed: $($_.Exception.Message)")
}

$result = [ordered]@{
    schema_version = 1
    check = "allocator-accounting-contract"
    inventory_path = $resolvedInventory
    expected_families = $knownFamilies.Count
    checked_families = $families.Count
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
