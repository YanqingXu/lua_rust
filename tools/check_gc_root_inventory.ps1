param(
    [string]$Root = "",
    [string]$InventoryPath = "tests/compatibility/gc_root_inventory.json",
    [string]$ResultPath = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$knownKinds = @(
    "COLLECTOR_EXPLICIT_ROOT",
    "GLOBAL_TABLE",
    "GLOBAL_ENVIRONMENTS",
    "REGISTRY",
    "PRIMITIVE_METATABLES",
    "MAIN_STATE_ENTRY",
    "RUNNING_THREAD",
    "MAIN_STACK",
    "COROUTINE_STACK",
    "CALL_FUNCTION",
    "ACTIVE_PROTO",
    "CALL_VARARGS",
    "OPEN_UPVALUES",
    "THREAD_CALLER_CHAIN",
    "DEBUG_HOOK",
    "DEBUG_PROTO",
    "YIELDED_VALUES",
    "LAST_ERROR",
    "PENDING_FINALIZERS",
    "TEMPORARY_PROTECTED_ROOTS",
    "TEMPORARY_STATE_ROOTS",
    "LIBRARY_LIVE_HANDLES",
    "FIXED_STRINGS"
)

$knownStatuses = @("implemented", "partial", "missing", "unsafe")
$failures = [System.Collections.Generic.List[string]]::new()
$checkedCount = 0
$inventorySchemaVersion = $null
$resolvedInventoryPath = ""

function Test-Property {
    param(
        [AllowNull()]
        [object]$Object,
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    return $null -ne $Object -and $Object.PSObject.Properties.Name -contains $Name
}

function Test-NonEmptyString {
    param([AllowNull()][object]$Value)
    return $Value -is [string] -and -not [string]::IsNullOrWhiteSpace($Value)
}

function Add-MissingStringFailure {
    param(
        [AllowNull()]
        [object]$Object,
        [Parameter(Mandatory = $true)]
        [string]$Property,
        [Parameter(Mandatory = $true)]
        [string]$Context
    )

    if (-not (Test-Property -Object $Object -Name $Property) -or
        -not (Test-NonEmptyString -Value $Object.$Property)) {
        $failures.Add("$Context requires a non-empty '$Property' string")
        return $false
    }
    return $true
}

try {
    if ([string]::IsNullOrWhiteSpace($Root)) {
        $Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    } else {
        $Root = (Resolve-Path -LiteralPath $Root).Path
    }

    $rootPrefix = $Root.TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    ) + [System.IO.Path]::DirectorySeparatorChar

    if ([System.IO.Path]::IsPathRooted($InventoryPath)) {
        $resolvedInventoryPath = [System.IO.Path]::GetFullPath($InventoryPath)
    } else {
        $resolvedInventoryPath = [System.IO.Path]::GetFullPath(
            (Join-Path $Root $InventoryPath)
        )
    }

    if (-not (Test-Path -LiteralPath $resolvedInventoryPath -PathType Leaf)) {
        $failures.Add("Inventory file does not exist: $resolvedInventoryPath")
    } else {
        try {
            $inventory = Get-Content -LiteralPath $resolvedInventoryPath -Raw |
                ConvertFrom-Json
        } catch {
            $failures.Add("Inventory is not valid JSON: $($_.Exception.Message)")
            $inventory = $null
        }

        if ($null -ne $inventory) {
            if (-not (Test-Property -Object $inventory -Name "schema_version")) {
                $failures.Add("Inventory requires schema_version")
            } else {
                $inventorySchemaVersion = $inventory.schema_version
                if (($inventorySchemaVersion -isnot [long] -and
                    $inventorySchemaVersion -isnot [int]) -or
                    $inventorySchemaVersion -ne 1) {
                    $failures.Add(
                        "Unsupported schema_version: $inventorySchemaVersion (expected 1)"
                    )
                }
            }

            [void](Add-MissingStringFailure `
                -Object $inventory `
                -Property "description" `
                -Context "inventory")

            if (-not (Test-Property -Object $inventory -Name "status_values")) {
                $failures.Add("Inventory requires status_values")
            } else {
                $declaredStatuses = @($inventory.status_values)
                if ($declaredStatuses.Count -ne $knownStatuses.Count) {
                    $failures.Add(
                        "Inventory status_values must contain exactly " +
                        "$($knownStatuses.Count) values"
                    )
                }
                foreach ($knownStatus in $knownStatuses) {
                    if (@($declaredStatuses | Where-Object {
                        $_ -eq $knownStatus
                    }).Count -ne 1) {
                        $failures.Add(
                            "Inventory status_values must contain '$knownStatus' exactly once"
                        )
                    }
                }
                foreach ($declaredStatus in $declaredStatuses) {
                    if (-not (Test-NonEmptyString -Value $declaredStatus) -or
                        $knownStatuses -notcontains $declaredStatus) {
                        $failures.Add(
                            "Inventory declares unknown status '$declaredStatus'"
                        )
                    }
                }
            }

            if (-not (Test-Property -Object $inventory -Name "root_kinds")) {
                $failures.Add("Inventory requires root_kinds")
                $entries = @()
            } else {
                $entries = @($inventory.root_kinds)
            }

            if ($entries.Count -eq 0) {
                $failures.Add("Inventory root_kinds must not be empty")
            }

            $seenKinds = @{}
            foreach ($entry in $entries) {
                $checkedCount += 1
                $entryContext = "root_kinds[$($checkedCount - 1)]"

                if (-not (Add-MissingStringFailure `
                    -Object $entry `
                    -Property "root_kind" `
                    -Context $entryContext)) {
                    continue
                }

                $kind = [string]$entry.root_kind
                $entryContext = "root_kind '$kind'"

                if ($knownKinds -notcontains $kind) {
                    $failures.Add("$entryContext is not a known RootKind")
                }
                if ($seenKinds.ContainsKey($kind)) {
                    $failures.Add("$entryContext is duplicated")
                } else {
                    $seenKinds[$kind] = $true
                }

                if (-not (Add-MissingStringFailure `
                    -Object $entry `
                    -Property "status" `
                    -Context $entryContext)) {
                    $status = ""
                } else {
                    $status = [string]$entry.status
                    if ($knownStatuses -notcontains $status) {
                        $failures.Add(
                            "$entryContext has unknown status '$status'"
                        )
                    }
                }

                [void](Add-MissingStringFailure `
                    -Object $entry `
                    -Property "missing_risk" `
                    -Context $entryContext)

                if (-not (Test-Property -Object $entry -Name "owner") -or
                    $null -eq $entry.owner) {
                    $failures.Add("$entryContext requires owner")
                } else {
                    [void](Add-MissingStringFailure `
                        -Object $entry.owner `
                        -Property "current" `
                        -Context "$entryContext owner")
                    [void](Add-MissingStringFailure `
                        -Object $entry.owner `
                        -Property "target" `
                        -Context "$entryContext owner")
                }

                if (-not (Test-Property -Object $entry -Name "tracer") -or
                    $null -eq $entry.tracer) {
                    $failures.Add("$entryContext requires tracer")
                } else {
                    [void](Add-MissingStringFailure `
                        -Object $entry.tracer `
                        -Property "current" `
                        -Context "$entryContext tracer")
                    [void](Add-MissingStringFailure `
                        -Object $entry.tracer `
                        -Property "target" `
                        -Context "$entryContext tracer")
                }

                if (-not (Test-Property -Object $entry -Name "planned_test") -or
                    $null -eq $entry.planned_test) {
                    $failures.Add("$entryContext requires planned_test")
                } else {
                    foreach ($testProperty in @("name", "layer", "assertion")) {
                        [void](Add-MissingStringFailure `
                            -Object $entry.planned_test `
                            -Property $testProperty `
                            -Context "$entryContext planned_test")
                    }
                }

                if (-not (Test-Property -Object $entry -Name "source_locations")) {
                    $failures.Add("$entryContext requires source_locations")
                    $locations = @()
                } else {
                    $locations = @($entry.source_locations)
                }

                if ($locations.Count -eq 0) {
                    $failures.Add(
                        "$entryContext requires at least one source location"
                    )
                }

                for ($locationIndex = 0;
                    $locationIndex -lt $locations.Count;
                    $locationIndex += 1) {
                    $location = $locations[$locationIndex]
                    $locationContext =
                        "$entryContext source_locations[$locationIndex]"

                    $hasPath = Add-MissingStringFailure `
                        -Object $location `
                        -Property "path" `
                        -Context $locationContext
                    [void](Add-MissingStringFailure `
                        -Object $location `
                        -Property "symbol" `
                        -Context $locationContext)

                    if (-not (Test-Property -Object $location -Name "line") -or
                        $location.line -isnot [long] -and
                        $location.line -isnot [int]) {
                        $failures.Add(
                            "$locationContext requires an integer 'line'"
                        )
                        $lineNumber = 0
                    } else {
                        $lineNumber = [long]$location.line
                        if ($lineNumber -lt 1) {
                            $failures.Add(
                                "$locationContext line must be positive"
                            )
                        }
                    }

                    if ($hasPath) {
                        $sourcePath = [System.IO.Path]::GetFullPath(
                            (Join-Path $Root ([string]$location.path))
                        )
                        if (-not $sourcePath.StartsWith(
                            $rootPrefix,
                            [System.StringComparison]::OrdinalIgnoreCase
                        )) {
                            $failures.Add(
                                "$locationContext escapes repository root"
                            )
                        } elseif (-not (
                            Test-Path -LiteralPath $sourcePath -PathType Leaf
                        )) {
                            $failures.Add(
                                "$locationContext source does not exist: " +
                                "$($location.path)"
                            )
                        } elseif ($lineNumber -gt 0) {
                            $lineCount = @(
                                Get-Content -LiteralPath $sourcePath
                            ).Count
                            if ($lineNumber -gt $lineCount) {
                                $failures.Add(
                                    "$locationContext line $lineNumber exceeds " +
                                    "$($location.path) line count $lineCount"
                                )
                            }
                        }
                    }
                }
            }

            foreach ($knownKind in $knownKinds) {
                if (-not $seenKinds.ContainsKey($knownKind)) {
                    $failures.Add(
                        "Inventory is missing known RootKind '$knownKind'"
                    )
                }
            }
        }
    }
} catch {
    $failures.Add("Validator failed closed: $($_.Exception.Message)")
}

$result = [ordered]@{
    schema_version = 1
    check = "gc-root-inventory"
    inventory_path = $resolvedInventoryPath
    inventory_schema_version = $inventorySchemaVersion
    expected_root_kinds = $knownKinds.Count
    checked_root_kinds = $checkedCount
    valid = $failures.Count -eq 0
    failures = @($failures)
}

$json = $result | ConvertTo-Json -Depth 8
if (-not [string]::IsNullOrWhiteSpace($ResultPath)) {
    $resolvedResultPath = if ([System.IO.Path]::IsPathRooted($ResultPath)) {
        [System.IO.Path]::GetFullPath($ResultPath)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $Root $ResultPath))
    }
    $resultDirectory = Split-Path -Parent $resolvedResultPath
    if (-not [string]::IsNullOrWhiteSpace($resultDirectory)) {
        [System.IO.Directory]::CreateDirectory($resultDirectory) | Out-Null
    }
    Set-Content -LiteralPath $resolvedResultPath -Value $json -Encoding utf8
}

Write-Output $json
if ($failures.Count -ne 0) {
    exit 1
}
exit 0
