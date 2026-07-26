param(
    [string]$Root = "",
    [string]$OraclePath = "tests/compatibility/oracle.toml",
    [string]$CppRoot = "",
    [string]$ResultPath = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
} else {
    $Root = (Resolve-Path -LiteralPath $Root).Path
}

function Resolve-RootedPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [string]$Base = $Root
    )

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $Base $Path))
}

function Get-TomlValue {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Text,
        [string]$Section = "",
        [Parameter(Mandatory = $true)]
        [string]$Key
    )

    $currentSection = ""
    foreach ($rawLine in ($Text -split "`n")) {
        $line = $rawLine.Trim()
        if ($line.Length -eq 0 -or $line.StartsWith("#")) {
            continue
        }
        if ($line -match '^\[([A-Za-z0-9_.-]+)\]$') {
            $currentSection = $Matches[1]
            continue
        }
        if ($currentSection -eq $Section -and
            $line -match ('^' + [regex]::Escape($Key) + '\s*=\s*(.+?)\s*$')) {
            $value = $Matches[1]
            if ($value -match '^"(.*)"$') {
                return $Matches[1]
            }
            if ($value -match '^\d+$') {
                return [int]$value
            }
            if ($value -eq "true") {
                return $true
            }
            if ($value -eq "false") {
                return $false
            }
            if ($value -match '^\[(.*)\]$') {
                $inner = $Matches[1].Trim()
                if ($inner.Length -eq 0) {
                    return @()
                }
                return @($inner -split ',' | ForEach-Object {
                    $item = $_.Trim()
                    if ($item -notmatch '^"(.*)"$') {
                        throw "Unsupported TOML array value for [$Section].$Key"
                    }
                    $Matches[1]
                })
            }
            throw "Unsupported TOML value for [$Section].$Key"
        }
    }
    throw "Missing TOML value [$Section].$Key"
}

function Get-Sha256 {
    param([Parameter(Mandatory = $true)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-GitBlobSha256 {
    param([Parameter(Mandatory = $true)][string]$RelativePath)

    $blob = (& git -C $Root rev-parse "HEAD:$RelativePath" 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($blob)) {
        throw "Official fixture is not tracked at HEAD: $RelativePath"
    }

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = "git"
    if ($startInfo.PSObject.Properties.Name -contains "ArgumentList") {
        $startInfo.ArgumentList.Add("-C")
        $startInfo.ArgumentList.Add($Root)
        $startInfo.ArgumentList.Add("cat-file")
        $startInfo.ArgumentList.Add("blob")
        $startInfo.ArgumentList.Add($blob)
    } else {
        $escapedRoot = $Root.Replace('"', '\"')
        $startInfo.Arguments = "-C `"$escapedRoot`" cat-file blob $blob"
    }
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $memory = [System.IO.MemoryStream]::new()
    try {
        if (-not $process.Start()) {
            throw "Could not start git cat-file"
        }
        $process.StandardOutput.BaseStream.CopyTo($memory)
        $stderr = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) {
            throw "git cat-file failed for ${RelativePath}: $stderr"
        }

        $sha = [System.Security.Cryptography.SHA256]::Create()
        try {
            return ([System.BitConverter]::ToString(
                $sha.ComputeHash($memory.ToArray())
            )).Replace("-", "").ToLowerInvariant()
        } finally {
            $sha.Dispose()
        }
    } finally {
        $memory.Dispose()
        $process.Dispose()
    }
}

function Normalize-GitUrl {
    param([string]$Url)
    if ([string]::IsNullOrWhiteSpace($Url)) {
        return ""
    }
    return $Url.Trim().TrimEnd("/").ToLowerInvariant() -replace '\.git$', ''
}

$oracleFile = Resolve-RootedPath $OraclePath
if (-not (Test-Path -LiteralPath $oracleFile -PathType Leaf)) {
    throw "Missing oracle configuration: $oracleFile"
}
$oracleText = Get-Content -LiteralPath $oracleFile -Raw

$schemaVersion = Get-TomlValue -Text $oracleText -Key "schema_version"
$reportSchemaVersion = Get-TomlValue -Text $oracleText -Key "differential_report_schema_version"
$cppRepository = Get-TomlValue -Text $oracleText -Section "cpp" -Key "repository"
$cppCommit = Get-TomlValue -Text $oracleText -Section "cpp" -Key "commit"
$luaRelease = Get-TomlValue -Text $oracleText -Section "lua" -Key "release"
$luaSourceUrl = Get-TomlValue -Text $oracleText -Section "lua" -Key "source_archive_url"
$luaSourceSha = Get-TomlValue -Text $oracleText -Section "lua" -Key "source_archive_sha256"
$suiteArchiveUrl = Get-TomlValue -Text $oracleText -Section "official_suite" -Key "archive_url"
$suiteArchiveSha = Get-TomlValue -Text $oracleText -Section "official_suite" -Key "archive_sha256"
$suiteManifestRelative = Get-TomlValue -Text $oracleText -Section "official_suite" -Key "manifest"
$suiteRootRelative = Get-TomlValue -Text $oracleText -Section "official_suite" -Key "vendored_root"
$suitePolicy = Get-TomlValue -Text $oracleText -Section "official_suite" -Key "vendored_policy"
$differentialManifestRelative = Get-TomlValue -Text $oracleText -Section "differential" -Key "manifest"
$versionProbeRelative = Get-TomlValue -Text $oracleText -Section "differential" -Key "version_probe"
$requiredLanes = @(Get-TomlValue -Text $oracleText -Section "differential" -Key "required_lanes")
$officialBuildScriptRelative = Get-TomlValue `
    -Text $oracleText `
    -Section "provisioning" `
    -Key "official_lua_build_script"
$cppBuildScriptRelative = Get-TomlValue `
    -Text $oracleText `
    -Section "provisioning" `
    -Key "cpp_build_script"

$failures = [System.Collections.Generic.List[string]]::new()
$warnings = [System.Collections.Generic.List[string]]::new()

if ($schemaVersion -ne 1) {
    $failures.Add("Unsupported oracle schema_version: $schemaVersion")
}
if ($reportSchemaVersion -ne 1) {
    $failures.Add("Unsupported differential report schema: $reportSchemaVersion")
}
if ($cppCommit -notmatch '^[0-9a-f]{40}$') {
    $failures.Add("C++ oracle must be a full 40-character commit SHA")
}
if ($requiredLanes.Count -ne 2 -or
    $requiredLanes -notcontains "official-lua51" -or
    $requiredLanes -notcontains "cpp-87c15e6") {
    $failures.Add("Differential lanes must lock official-lua51 and cpp-87c15e6")
}
foreach ($buildScript in @($officialBuildScriptRelative, $cppBuildScriptRelative)) {
    $buildScriptPath = Resolve-RootedPath $buildScript
    if (-not (Test-Path -LiteralPath $buildScriptPath -PathType Leaf)) {
        $failures.Add("Missing oracle provisioning script: $buildScript")
    }
}

if ([string]::IsNullOrWhiteSpace($CppRoot)) {
    if (-not [string]::IsNullOrWhiteSpace($env:LUA_CPP_ORACLE_ROOT)) {
        $CppRoot = $env:LUA_CPP_ORACLE_ROOT
    } else {
        $CppRoot = Join-Path (Split-Path -Parent $Root) "lua_cpp"
    }
}
$cppPath = Resolve-RootedPath -Path $CppRoot
$actualCppCommit = ""
$actualCppRepository = ""
if (-not (Test-Path -LiteralPath (Join-Path $cppPath ".git"))) {
    $failures.Add("Missing pinned C++ oracle checkout: $cppPath")
} else {
    $actualCppCommit = (& git -C $cppPath rev-parse HEAD 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        $failures.Add("Could not read C++ oracle commit: $cppPath")
    } elseif ($actualCppCommit -ne $cppCommit) {
        $failures.Add("C++ oracle commit mismatch: expected $cppCommit, got $actualCppCommit")
    }

    $actualCppRepository = (& git -C $cppPath remote get-url origin 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        $failures.Add("Could not read C++ oracle origin URL: $cppPath")
    } elseif ((Normalize-GitUrl $actualCppRepository) -ne (Normalize-GitUrl $cppRepository)) {
        $failures.Add("C++ oracle repository mismatch: expected $cppRepository, got $actualCppRepository")
    }
}

$manifestFile = Resolve-RootedPath $suiteManifestRelative
$suiteRoot = Resolve-RootedPath $suiteRootRelative
$manifest = $null
$vendoredCount = 0
$upstreamCount = 0
$missingSupport = @()
if (-not (Test-Path -LiteralPath $manifestFile -PathType Leaf)) {
    $failures.Add("Missing official suite manifest: $manifestFile")
} elseif (-not (Test-Path -LiteralPath $suiteRoot -PathType Container)) {
    $failures.Add("Missing vendored official suite root: $suiteRoot")
} else {
    $manifest = Get-Content -LiteralPath $manifestFile -Raw | ConvertFrom-Json
    if ($manifest.schemaVersion -ne 2) {
        $failures.Add("Unsupported official source manifest schema: $($manifest.schemaVersion)")
    }
    if ($manifest.archive.url -ne $suiteArchiveUrl -or
        $manifest.archive.sha256 -ne $suiteArchiveSha -or
        $manifest.referenceCompiler.release -ne $luaRelease -or
        $manifest.referenceCompiler.sourceArchiveUrl -ne $luaSourceUrl -or
        $manifest.referenceCompiler.sourceArchiveSha256 -ne $luaSourceSha) {
        $failures.Add("Official source manifest provenance differs from oracle.toml")
    }

    $fixturePath = Resolve-RootedPath $manifest.referenceCompiler.oracleFixture
    if (-not (Test-Path -LiteralPath $fixturePath -PathType Leaf)) {
        $failures.Add("Missing compiler oracle fixture: $fixturePath")
    } elseif ((Get-Sha256 $fixturePath) -ne $manifest.referenceCompiler.oracleFixtureSha256) {
        $failures.Add("Compiler oracle fixture hash mismatch")
    }

    $entriesByPath = @{}
    foreach ($entry in @($manifest.entries)) {
        $upstreamCount++
        if ($entriesByPath.ContainsKey($entry.path)) {
            $failures.Add("Duplicate official source path: $($entry.path)")
        } else {
            $entriesByPath[$entry.path] = $entry.sha256
        }
    }

    $suitePrefix = $suiteRoot.TrimEnd("\", "/") +
        [System.IO.Path]::DirectorySeparatorChar
    $vendoredFiles = @(Get-ChildItem -LiteralPath $suiteRoot -Recurse -Force -File)
    foreach ($file in $vendoredFiles) {
        $relativeInSuite = $file.FullName.Substring($suitePrefix.Length).Replace("\", "/")
        if (-not $entriesByPath.ContainsKey($relativeInSuite)) {
            $failures.Add("Unclassified official suite file: $relativeInSuite")
            continue
        }
        $relativeInRepo = "$suiteRootRelative/$relativeInSuite"
        try {
            $actualHash = Get-GitBlobSha256 $relativeInRepo
            if ($actualHash -ne $entriesByPath[$relativeInSuite]) {
                $failures.Add("Official suite hash mismatch: $relativeInSuite")
            }
        } catch {
            $failures.Add($_.Exception.Message)
        }
        $vendoredCount++
    }

    foreach ($entry in @($manifest.entries)) {
        $candidate = Join-Path $suiteRoot ($entry.path.Replace("/", "\"))
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            $missingSupport += $entry.path
        }
    }
    if ($suitePolicy -eq "strict-complete" -and $missingSupport.Count -gt 0) {
        $failures.Add("Official suite is incomplete: $($missingSupport -join ', ')")
    } elseif ($suitePolicy -eq "tracked-subset" -and $missingSupport.Count -gt 0) {
        $warnings.Add(
            "Vendored official suite is a locked subset; missing support files: " +
            ($missingSupport -join ", ")
        )
    } elseif ($suitePolicy -notin @("strict-complete", "tracked-subset")) {
        $failures.Add("Unsupported official suite vendored_policy: $suitePolicy")
    }
}

$differentialManifestFile = Resolve-RootedPath $differentialManifestRelative
$versionProbeFile = Resolve-RootedPath $versionProbeRelative
$differentialCount = 0
if (-not (Test-Path -LiteralPath $versionProbeFile -PathType Leaf)) {
    $failures.Add("Missing _VERSION differential probe: $versionProbeFile")
}
if (-not (Test-Path -LiteralPath $differentialManifestFile -PathType Leaf)) {
    $failures.Add("Missing differential manifest: $differentialManifestFile")
} else {
    $differentialManifest = Get-Content -LiteralPath $differentialManifestFile -Raw |
        ConvertFrom-Json
    if ($differentialManifest.schemaVersion -ne 1) {
        $failures.Add(
            "Unsupported differential case schema: $($differentialManifest.schemaVersion)"
        )
    }
    foreach ($channel in @("stdout", "stderr")) {
        $rules = @($differentialManifest.normalization.$channel)
        if (($rules -join ",") -ne "crlf-to-lf") {
            $failures.Add(
                "Differential $channel normalization must be exactly crlf-to-lf"
            )
        }
    }
    $deviationLogPath = Resolve-RootedPath "docs/rust_migration/deviation_log.md"
    $deviationLog = if (Test-Path -LiteralPath $deviationLogPath -PathType Leaf) {
        Get-Content -LiteralPath $deviationLogPath -Raw
    } else {
        ""
    }
    foreach ($expectedDifference in @($differentialManifest.expectedDifferences)) {
        if ([string]::IsNullOrWhiteSpace($expectedDifference.deviation)) {
            $failures.Add(
                "Expected difference '$($expectedDifference.id)' lacks a deviation ID"
            )
        } elseif ($deviationLog -notmatch
            ('(?m)^###\s+' + [regex]::Escape($expectedDifference.deviation) + ':')) {
            $failures.Add(
                "Expected difference '$($expectedDifference.id)' references missing " +
                "deviation $($expectedDifference.deviation)"
            )
        }
    }
    $caseIds = @{}
    foreach ($case in @($differentialManifest.cases)) {
        $differentialCount++
        if ($caseIds.ContainsKey($case.id)) {
            $failures.Add("Duplicate differential case id: $($case.id)")
        } else {
            $caseIds[$case.id] = $true
        }
        $scriptPath = Resolve-RootedPath $case.script
        if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
            $failures.Add("Missing differential case script: $($case.script)")
        }
        foreach ($requiredEvidence in @("stdout", "stderr", "exitStatus")) {
            if (@($case.evidence) -notcontains $requiredEvidence) {
                $failures.Add(
                    "Differential case '$($case.id)' lacks $requiredEvidence evidence"
                )
            }
        }
    }
    if ($differentialCount -ne 4) {
        $failures.Add("Expected exactly 4 differential cases, got $differentialCount")
    }
}

$document = [ordered]@{
    schemaVersion = 1
    channel = "oracle-baseline"
    passed = $failures.Count -eq 0
    oracle = [ordered]@{
        path = $OraclePath
        sha256 = Get-Sha256 $oracleFile
    }
    cpp = [ordered]@{
        repository = $cppRepository
        expectedCommit = $cppCommit
        checkout = $cppPath
        actualRepository = $actualCppRepository
        actualCommit = $actualCppCommit
    }
    lua = [ordered]@{
        release = $luaRelease
        sourceArchiveSha256 = $luaSourceSha
        officialSuiteArchiveSha256 = $suiteArchiveSha
        upstreamFileCount = $upstreamCount
        vendoredFileCount = $vendoredCount
        missingSupportFiles = @($missingSupport)
    }
    differential = [ordered]@{
        schemaVersion = $reportSchemaVersion
        lanes = @($requiredLanes)
        caseCount = $differentialCount
    }
    warnings = @($warnings)
    failures = @($failures)
}

if (-not [string]::IsNullOrWhiteSpace($ResultPath)) {
    $resolvedResultPath = Resolve-RootedPath $ResultPath
    $parent = Split-Path -Parent $resolvedResultPath
    if (-not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    $json = $document | ConvertTo-Json -Depth 8
    [System.IO.File]::WriteAllText(
        $resolvedResultPath,
        $json + [Environment]::NewLine,
        [System.Text.UTF8Encoding]::new($false)
    )
}

Write-Host "[INFO] C++ oracle: $actualCppCommit ($actualCppRepository)"
Write-Host "[INFO] Lua oracle: $luaRelease / source $luaSourceSha"
Write-Host "[INFO] Official suite: $vendoredCount/$upstreamCount files vendored"
Write-Host "[INFO] Differential schema/cases: $reportSchemaVersion/$differentialCount"
$warnings | ForEach-Object { Write-Warning $_ }

if ($failures.Count -gt 0) {
    Write-Host "[FAIL] Oracle baseline validation failed:"
    $failures | ForEach-Object { Write-Host " - $_" }
    exit 1
}

Write-Host "[OK] Oracle baseline is pinned and auditable"
