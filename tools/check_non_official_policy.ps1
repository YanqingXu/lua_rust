<#
.SYNOPSIS
    Reject non-official regressions outside the reviewed case/channel signatures.
#>
param(
    [Parameter(Mandatory = $true)][string]$ArtifactPath,
    [string]$ManifestPath = "",
    [string]$PolicyPath = "",
    [string]$ResultPath = "target/compatibility/non-official-policy.json"
)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot "non_official_policy.ps1")
$root = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($ManifestPath)) {
    $ManifestPath = Join-Path $root "tests/compatibility/lua_fixtures.json"
}
if ([string]::IsNullOrWhiteSpace($PolicyPath)) {
    $PolicyPath = Join-Path $root "tests/compatibility/non-official-debt.json"
}
try {
    $result = Test-NonOfficialArtifact `
        -Artifact (Get-Content -LiteralPath $ArtifactPath -Raw | ConvertFrom-Json) `
        -Manifest (Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json) `
        -Policy (Get-Content -LiteralPath $PolicyPath -Raw | ConvertFrom-Json)
} catch {
    $result = [pscustomobject]@{
        schemaVersion = 1; channel = "non-official-policy"; passed = $false
        acceptedDebt = @(); resolvedDebt = @(); failures = @($_.Exception.Message)
    }
}
$result | Add-Member -NotePropertyName policyPath -NotePropertyValue $PolicyPath
$result | Add-Member -NotePropertyName artifactPath -NotePropertyValue $ArtifactPath
$resultFile = [System.IO.Path]::GetFullPath($ResultPath)
$null = New-Item -ItemType Directory -Path (Split-Path -Parent $resultFile) -Force
[System.IO.File]::WriteAllText($resultFile,
    ($result | ConvertTo-Json -Depth 12) + [Environment]::NewLine,
    [System.Text.UTF8Encoding]::new($false))
if (-not $result.passed) {
    $result.failures | ForEach-Object { Write-Host "[FAIL] $_" }
    exit 1
}
Write-Host "[OK] Non-official policy: $(@($result.acceptedDebt).Count) bounded debts; $(@($result.resolvedDebt).Count) resolved debts"
if (@($result.resolvedDebt).Count -gt 0) {
    Write-Host "[INFO] Remove resolved entries when landing their fixes: $($result.resolvedDebt -join ', ')"
}
exit 0
