param(
    [string]$Root = "",
    [string]$BaseRef = "",
    [string]$Labels = "",
    [string]$OraclePath = "tests/compatibility/oracle.toml",
    [string]$SummaryDirectory = "docs/rust_migration/oracle_baseline_changes",
    [string]$ResultPath = "target/compatibility/oracle-change.json"
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

function Get-ConfiguredRequiredLabel {
    param([Parameter(Mandatory = $true)][string]$Path)
    $text = Get-Content -LiteralPath $Path -Raw
    $inSection = $false
    foreach ($rawLine in ($text -split "`n")) {
        $line = $rawLine.Trim()
        if ($line -match '^\[([A-Za-z0-9_.-]+)\]$') {
            $inSection = $Matches[1] -eq "baseline_change"
            continue
        }
        if ($inSection -and $line -match '^required_label\s*=\s*"([^"]+)"$') {
            return $Matches[1]
        }
    }
    throw "Missing [baseline_change].required_label in $Path"
}

function ConvertTo-LabelList {
    param([string]$Value)
    if ([string]::IsNullOrWhiteSpace($Value)) {
        return @()
    }
    $trimmed = $Value.Trim()
    if ($trimmed.StartsWith("[")) {
        $parsed = $trimmed | ConvertFrom-Json
        return @($parsed | ForEach-Object {
            if ($_ -is [string]) {
                $_
            } elseif ($null -ne $_.name) {
                $_.name
            }
        })
    }
    return @($trimmed -split "," | ForEach-Object { $_.Trim() } |
        Where-Object { $_.Length -gt 0 })
}

if ([string]::IsNullOrWhiteSpace($BaseRef)) {
    $BaseRef = $env:GITHUB_BASE_REF
}

$oracleFile = Resolve-RootedPath $OraclePath
if (-not (Test-Path -LiteralPath $oracleFile -PathType Leaf)) {
    throw "Missing oracle configuration: $oracleFile"
}
$requiredLabel = Get-ConfiguredRequiredLabel $oracleFile
$labelList = @(ConvertTo-LabelList $Labels)

$status = "not-applicable"
$changedOracleFiles = @()
$changedSummaryFiles = @()
$diffSummary = ""
$failures = [System.Collections.Generic.List[string]]::new()
$resolvedBase = ""

if (-not [string]::IsNullOrWhiteSpace($BaseRef)) {
    foreach ($candidate in @("origin/$BaseRef", $BaseRef)) {
        $null = & git -C $Root rev-parse --verify "$candidate^{commit}" 2>$null
        if ($LASTEXITCODE -eq 0) {
            $resolvedBase = $candidate
            break
        }
    }
    if ([string]::IsNullOrWhiteSpace($resolvedBase)) {
        $failures.Add("Could not resolve baseline comparison ref: $BaseRef")
        $status = "infrastructure-error"
    } else {
        $trackedOraclePaths = @(
            $OraclePath,
            "tests/compatibility/lua51-official-sources.json",
            "tests/compatibility/lua51-loadnil-oracle.lua",
            "tests/compatibility/lua51-differential-cases.json",
            "tests/compatibility/lua51-version-probe.lua",
            "tests/lua/differential",
            "tests/lua/official"
        )
        $changedOracleFiles = @(
            & git -C $Root diff --name-only "$resolvedBase...HEAD" -- @trackedOraclePaths
        )
        if ($LASTEXITCODE -ne 0) {
            $failures.Add("git diff failed while checking oracle changes")
            $status = "infrastructure-error"
        } elseif ($changedOracleFiles.Count -eq 0) {
            $status = "unchanged"
        } else {
            $status = "baseline-change"
            $changedSummaryFiles = @(
                & git -C $Root diff --name-only "$resolvedBase...HEAD" -- $SummaryDirectory
            ) | Where-Object { $_ -match '\.md$' }
            $diffSummary = (
                & git -C $Root diff --stat "$resolvedBase...HEAD" -- @trackedOraclePaths
            ) -join "`n"

            if ($labelList -notcontains $requiredLabel) {
                $failures.Add(
                    "Oracle files changed without required PR label '$requiredLabel'"
                )
            }
            if ($changedSummaryFiles.Count -eq 0) {
                $failures.Add(
                    "Oracle files changed without a Markdown diff summary under " +
                    "$SummaryDirectory"
                )
            }
        }
    }
}

$document = [ordered]@{
    schemaVersion = 1
    channel = "oracle-change"
    passed = $failures.Count -eq 0
    status = $status
    baseRef = $BaseRef
    resolvedBase = $resolvedBase
    requiredLabel = $requiredLabel
    labels = @($labelList)
    changedOracleFiles = @($changedOracleFiles)
    changedSummaryFiles = @($changedSummaryFiles)
    diffSummary = $diffSummary
    failures = @($failures)
}

$resolvedResultPath = Resolve-RootedPath $ResultPath
$resultParent = Split-Path -Parent $resolvedResultPath
if (-not (Test-Path -LiteralPath $resultParent)) {
    New-Item -ItemType Directory -Path $resultParent -Force | Out-Null
}
$json = $document | ConvertTo-Json -Depth 6
[System.IO.File]::WriteAllText(
    $resolvedResultPath,
    $json + [Environment]::NewLine,
    [System.Text.UTF8Encoding]::new($false)
)

Write-Host "[INFO] Oracle change status: $status"
if ($changedOracleFiles.Count -gt 0) {
    Write-Host "[INFO] Changed oracle files: $($changedOracleFiles -join ', ')"
}
if (-not [string]::IsNullOrWhiteSpace($diffSummary)) {
    Write-Host $diffSummary
}
if ($failures.Count -gt 0) {
    Write-Host "[FAIL] Oracle baseline-change policy failed:"
    $failures | ForEach-Object { Write-Host " - $_" }
    exit 1
}

Write-Host "[OK] Oracle baseline-change policy satisfied"
