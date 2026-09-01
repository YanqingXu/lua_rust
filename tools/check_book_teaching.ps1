$ErrorActionPreference = 'Stop'

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$versionsRoot = Join-Path $repositoryRoot 'book/src/versions'
$requiredMarkers = @(
    '<a id="learning-task-card"></a>',
    '<a id="learning-predict"></a>',
    '<a id="learning-code-map"></a>',
    '<a id="learning-exercises"></a>',
    '<a id="learning-checkpoint"></a>'
)
$exerciseClasses = @(
    'exercise understanding',
    'exercise implementation',
    'exercise debugging'
)
$maximumAnchorLines = 60
$failures = [System.Collections.Generic.List[string]]::new()
$pages = Get-ChildItem -LiteralPath $versionsRoot -File -Filter 'v*.md' |
    Sort-Object Name

function Resolve-IncludeAnchor {
    param(
        [Parameter(Mandatory)] [string] $PagePath,
        [Parameter(Mandatory)] [string] $IncludePath,
        [Parameter(Mandatory)] [string] $Anchor
    )

    $normalizedIncludePath = $IncludePath -replace '/', [IO.Path]::DirectorySeparatorChar
    $sourcePath = [IO.Path]::GetFullPath(
        (Join-Path (Split-Path -Parent $PagePath) $normalizedIncludePath)
    )
    if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
        return [pscustomobject]@{ Error = "missing include source $sourcePath" }
    }

    $sourceLines = Get-Content -LiteralPath $sourcePath
    $startMatches = @($sourceLines | Select-String -SimpleMatch "ANCHOR: $Anchor")
    $endMatches = @($sourceLines | Select-String -SimpleMatch "ANCHOR_END: $Anchor")
    if ($startMatches.Count -ne 1 -or $endMatches.Count -ne 1) {
        return [pscustomobject]@{
            Error = "anchor '$Anchor' must have exactly one start and end in $sourcePath"
        }
    }

    $lineCount = $endMatches[0].LineNumber - $startMatches[0].LineNumber - 1
    if ($lineCount -lt 0) {
        return [pscustomobject]@{ Error = "anchor '$Anchor' ends before it starts in $sourcePath" }
    }
    return [pscustomobject]@{ Path = $sourcePath; Lines = $lineCount }
}

foreach ($page in $pages) {
    $lines = Get-Content -LiteralPath $page.FullName
    $content = $lines -join "`n"

    foreach ($marker in $requiredMarkers) {
        if (-not $content.Contains($marker)) {
            $failures.Add("$($page.Name): missing contract marker $marker")
        }
    }
    foreach ($className in $exerciseClasses) {
        if ($content -notmatch ('class="' + [regex]::Escape($className) + '"')) {
            $failures.Add("$($page.Name): missing exercise class '$className'")
        }
    }

    foreach ($line in $lines) {
        if ($line -notmatch '\{\{#include\s+([^:}]+):([^}]+)\}\}') {
            continue
        }
        $result = Resolve-IncludeAnchor -PagePath $page.FullName `
            -IncludePath $Matches[1] -Anchor $Matches[2]
        if ($result.Error) {
            $failures.Add("$($page.Name): $($result.Error)")
        } elseif ($result.Lines -gt $maximumAnchorLines) {
            $relativeSource = $result.Path.Substring($repositoryRoot.Length).TrimStart('\', '/')
            $failures.Add(
                "$($page.Name): anchor '$($Matches[2])' has $($result.Lines) lines " +
                "in $relativeSource (maximum $maximumAnchorLines)"
            )
        }
    }
}

if ($failures.Count -gt 0) {
    Write-Error (
        "Teaching-book checks failed ($($failures.Count)):`n- " +
        ($failures -join "`n- ")
    )
}

Write-Host "Teaching-book checks passed for $($pages.Count) version pages."
