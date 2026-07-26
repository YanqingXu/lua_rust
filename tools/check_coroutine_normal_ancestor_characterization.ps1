[CmdletBinding()]
param(
    [string]$Root = "",
    [string]$ManifestPath =
        "tests/compatibility/coroutine-normal-ancestor-characterization.json",
    [string]$CppLua = "",
    [string]$CppRoot = "../lua_cpp",
    [string]$OfficialLua = "",
    [string]$CppBuildReport = "",
    [string]$OfficialBuildReport = "",
    [ValidateRange(1, 60)]
    [int]$TimeoutSeconds = 10
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
} else {
    $Root = (Resolve-Path -LiteralPath $Root).Path
}

function Resolve-RootedPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [string]$Role
    )

    $resolved = if ([System.IO.Path]::IsPathRooted($Path)) {
        [System.IO.Path]::GetFullPath($Path)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $Root $Path))
    }
    if (-not (Test-Path -LiteralPath $resolved)) {
        throw "Missing $Role`: $resolved"
    }
    return $resolved
}

function Get-Sha256 {
    param([byte[]]$Bytes)

    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([System.BitConverter]::ToString(
            $sha.ComputeHash($Bytes)
        )).Replace("-", "").ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function Get-FileSha256 {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    return (
        Get-FileHash -LiteralPath $Path -Algorithm SHA256
    ).Hash.ToLowerInvariant()
}

function Normalize-LineEndings {
    param([byte[]]$Bytes)

    $normalized = [System.Collections.Generic.List[byte]]::new($Bytes.Length)
    for ($index = 0; $index -lt $Bytes.Length; $index++) {
        if ($Bytes[$index] -eq 0x0d) {
            $normalized.Add(0x0a)
            if ($index + 1 -lt $Bytes.Length -and
                $Bytes[$index + 1] -eq 0x0a) {
                $index++
            }
        } else {
            $normalized.Add($Bytes[$index])
        }
    }
    return ,$normalized.ToArray()
}

function Test-ByteArrayEqual {
    param(
        [byte[]]$Left,
        [byte[]]$Right
    )

    if ($Left.Length -ne $Right.Length) {
        return $false
    }
    for ($index = 0; $index -lt $Left.Length; $index++) {
        if ($Left[$index] -ne $Right[$index]) {
            return $false
        }
    }
    return $true
}

function Invoke-Fixture {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Executable,
        [Parameter(Mandatory = $true)]
        [string]$Fixture
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    if ($Fixture.Contains('"')) {
        throw "Fixture path cannot contain a quote: $Fixture"
    }
    # ProcessStartInfo.ArgumentList and Process.Kill(true) are unavailable in
    # Windows PowerShell 5.1. A quoted single path is sufficient here because
    # Windows paths cannot contain a double quote.
    $startInfo.Arguments = '"' + $Fixture + '"'
    $startInfo.WorkingDirectory = $Root
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    [void]$startInfo.EnvironmentVariables.Remove("LUA_INIT")

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $stdout = [System.IO.MemoryStream]::new()
    $stderr = [System.IO.MemoryStream]::new()
    try {
        if (-not $process.Start()) {
            throw "Could not start oracle: $Executable"
        }
        $stdoutCopy = $process.StandardOutput.BaseStream.CopyToAsync($stdout)
        $stderrCopy = $process.StandardError.BaseStream.CopyToAsync($stderr)
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            if (-not $process.HasExited) {
                $process.Kill()
            }
            if (-not $process.WaitForExit(5000)) {
                throw "Oracle did not exit after termination: $Executable"
            }
            [void]$stdoutCopy.Wait(5000)
            [void]$stderrCopy.Wait(5000)
            throw "Oracle timed out after $TimeoutSeconds seconds: $Executable"
        }
        if (-not $stdoutCopy.Wait(5000) -or
            -not $stderrCopy.Wait(5000)) {
            throw "Oracle output pipes did not close: $Executable"
        }
        [void]$stdoutCopy.GetAwaiter().GetResult()
        [void]$stderrCopy.GetAwaiter().GetResult()
        [void]$process.WaitForExit()

        return [pscustomobject]@{
            exitCode = $process.ExitCode
            stdout = Normalize-LineEndings -Bytes $stdout.ToArray()
            stderr = Normalize-LineEndings -Bytes $stderr.ToArray()
        }
    } finally {
        $stdout.Dispose()
        $stderr.Dispose()
        $process.Dispose()
    }
}

function Assert-Stream {
    param(
        [Parameter(Mandatory = $true)]
        [string]$OracleId,
        [Parameter(Mandatory = $true)]
        [string]$StreamName,
        [Parameter(Mandatory = $true)]
        [object]$Expected,
        [byte[]]$Actual
    )

    [byte[]]$expectedBytes =
        [System.Convert]::FromBase64String([string]$Expected.base64)
    $expectedSha256 = Get-Sha256 -Bytes $expectedBytes
    if ($expectedBytes.Length -ne [int]$Expected.byteLength -or
        $expectedSha256 -cne [string]$Expected.sha256 -or
        [System.Text.Encoding]::UTF8.GetString($expectedBytes) -cne
            [string]$Expected.utf8) {
        throw "$OracleId $StreamName manifest evidence is internally inconsistent"
    }
    if (-not (Test-ByteArrayEqual -Left $expectedBytes -Right $Actual)) {
        $actualSha256 = Get-Sha256 -Bytes $Actual
        throw (
            "$OracleId $StreamName changed: expected {0}/{1} bytes, " +
            "observed {2}/{3} bytes" -f
            $Expected.sha256,
            $Expected.byteLength,
            $actualSha256,
            $Actual.Length
        )
    }
}

$manifestFile = Resolve-RootedPath -Path $ManifestPath -Role "manifest"
$manifest = Get-Content -LiteralPath $manifestFile -Raw | ConvertFrom-Json
if ([int]$manifest.schemaVersion -ne 1 -or
    [bool]$manifest.gate -or
    [string]$manifest.classification -cne "characterization-only" -or
    [string]$manifest.implementationStatus -cne "pending-lua-rust") {
    throw (
        "Manifest must be schema 1, characterization-only, non-gating, " +
        "and pending-lua-rust"
    )
}
if ($null -ne $manifest.PSObject.Properties["expectedDifferences"] -or
    $null -ne $manifest.PSObject.Properties["approvedDeviation"]) {
    throw "A characterization manifest cannot approve or expect a deviation"
}
$stdoutNormalization = @($manifest.normalization.stdout)
$stderrNormalization = @($manifest.normalization.stderr)
if (($stdoutNormalization -join ",") -cne "crlf-to-lf" -or
    ($stderrNormalization -join ",") -cne "crlf-to-lf") {
    throw "Characterization normalization must be exactly crlf-to-lf"
}

$fixture = Resolve-RootedPath -Path ([string]$manifest.fixture) -Role "fixture"
$cppRepository = Resolve-RootedPath -Path $CppRoot -Role "C++ oracle repository"

$observations = @($manifest.observations)
if ($observations.Count -ne 2) {
    throw "Characterization manifest must contain exactly two oracle observations"
}
$byId = @{}
foreach ($observation in $observations) {
    $id = [string]$observation.id
    if ($byId.ContainsKey($id)) {
        throw "Duplicate oracle observation: $id"
    }
    $byId[$id] = $observation
}
foreach ($requiredId in @("cpp-87c15e6", "official-lua51")) {
    if (-not $byId.ContainsKey($requiredId)) {
        throw "Missing oracle observation: $requiredId"
    }
}

$cppExpected = $byId["cpp-87c15e6"]
$officialExpected = $byId["official-lua51"]
if ([string]$cppExpected.role -cne "project-target" -or
    [string]$officialExpected.role -cne "secondary-reference") {
    throw "Oracle roles must identify the C++ project target and stock reference"
}

if ([string]::IsNullOrWhiteSpace($CppBuildReport)) {
    $CppBuildReport = [string]$manifest.provenance.cppBuildReport
}
if ([string]::IsNullOrWhiteSpace($OfficialBuildReport)) {
    $OfficialBuildReport = [string]$manifest.provenance.officialBuildReport
}
$cppBuildReportFile =
    Resolve-RootedPath -Path $CppBuildReport -Role "C++ oracle build report"
$officialBuildReportFile =
    Resolve-RootedPath -Path $OfficialBuildReport -Role "official Lua build report"
$cppBuild =
    Get-Content -LiteralPath $cppBuildReportFile -Raw | ConvertFrom-Json
$officialBuild =
    Get-Content -LiteralPath $officialBuildReportFile -Raw | ConvertFrom-Json

if ([int]$cppBuild.schemaVersion -ne 1 -or
    [string]$cppBuild.channel -cne "cpp-oracle-build" -or
    -not [bool]$cppBuild.passed -or
    [string]$cppBuild.repository -cne [string]$cppExpected.sourceRepository -or
    [string]$cppBuild.expectedCommit -cne [string]$cppExpected.sourceCommit -or
    [string]$cppBuild.actualCommit -cne [string]$cppExpected.sourceCommit) {
    throw "C++ oracle build report does not match the characterization provenance"
}
if ([int]$officialBuild.schemaVersion -ne 1 -or
    [string]$officialBuild.channel -cne "lua51-oracle-build" -or
    -not [bool]$officialBuild.passed -or
    [string]$officialBuild.source.url -cne
        [string]$officialExpected.sourceArchiveUrl -or
    [string]$officialBuild.source.expectedSha256 -cne
        [string]$officialExpected.sourceArchiveSha256 -or
    [string]$officialBuild.source.actualSha256 -cne
        [string]$officialExpected.sourceArchiveSha256 -or
    -not ([string]$officialBuild.executable.version).StartsWith(
        [string]$officialExpected.sourceRelease,
        [System.StringComparison]::Ordinal
    )) {
    throw "Official Lua build report does not match the characterization provenance"
}

$reportedCppExecutable = Resolve-RootedPath `
    -Path ([string]$cppBuild.executables.luaApp.path) `
    -Role "reported C++ oracle executable"
$reportedOfficialExecutable = Resolve-RootedPath `
    -Path ([string]$officialBuild.executable.path) `
    -Role "reported official Lua executable"
$reportedCppHash = [string]$cppBuild.executables.luaApp.sha256
$reportedOfficialHash = [string]$officialBuild.executable.sha256
if ((Get-FileSha256 -Path $reportedCppExecutable) -cne $reportedCppHash) {
    throw "Reported C++ oracle executable hash no longer matches its build report"
}
if ((Get-FileSha256 -Path $reportedOfficialExecutable) -cne
    $reportedOfficialHash) {
    throw "Reported official Lua executable hash no longer matches its build report"
}

$cppExecutable = if ([string]::IsNullOrWhiteSpace($CppLua)) {
    $reportedCppExecutable
} else {
    Resolve-RootedPath -Path $CppLua -Role "C++ oracle"
}
$officialExecutable = if ([string]::IsNullOrWhiteSpace($OfficialLua)) {
    $reportedOfficialExecutable
} else {
    Resolve-RootedPath -Path $OfficialLua -Role "official Lua oracle"
}
if ((Get-FileSha256 -Path $cppExecutable) -cne $reportedCppHash) {
    throw "Selected C++ oracle binary does not match the provenance build"
}
if ((Get-FileSha256 -Path $officialExecutable) -cne
    $reportedOfficialHash) {
    throw "Selected official Lua binary does not match the provenance build"
}

$cppCommit = (& git -C $cppRepository rev-parse HEAD 2>$null | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or
    $cppCommit -cne [string]$cppExpected.sourceCommit) {
    throw (
        "C++ oracle commit mismatch: expected {0}, observed {1}" -f
        $cppExpected.sourceCommit,
        $cppCommit
    )
}
$cppOrigin = (& git -C $cppRepository remote get-url origin 2>$null | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or
    $cppOrigin.TrimEnd("/") -cne
        ([string]$cppExpected.sourceRepository).TrimEnd("/")) {
    throw (
        "C++ oracle repository mismatch: expected {0}, observed {1}" -f
        $cppExpected.sourceRepository,
        $cppOrigin
    )
}

$executables = @{
    "cpp-87c15e6" = $cppExecutable
    "official-lua51" = $officialExecutable
}
$runs = [int]$manifest.stabilityRuns
if ($runs -lt 1) {
    throw "stabilityRuns must be positive"
}

foreach ($id in @("cpp-87c15e6", "official-lua51")) {
    $expected = $byId[$id]
    for ($run = 1; $run -le $runs; $run++) {
        $actual = Invoke-Fixture -Executable $executables[$id] -Fixture $fixture
        if ($actual.exitCode -ne [int]$expected.expectedExit) {
            throw (
                "$id run $run exit changed: expected {0}, observed {1}" -f
                $expected.expectedExit,
                $actual.exitCode
            )
        }
        Assert-Stream -OracleId $id -StreamName "stdout" `
            -Expected $expected.stdout -Actual $actual.stdout
        Assert-Stream -OracleId $id -StreamName "stderr" `
            -Expected $expected.stderr -Actual $actual.stderr
    }
}

[ordered]@{
    status = "characterization-passed"
    gate = $false
    fixture = [string]$manifest.fixture
    stabilityRuns = $runs
    provenance = [ordered]@{
        cppBuildReport = $cppBuildReportFile
        cppExecutableSha256 = $reportedCppHash
        officialBuildReport = $officialBuildReportFile
        officialExecutableSha256 = $reportedOfficialHash
    }
    observations = @($observations | ForEach-Object {
        [ordered]@{
            id = $_.id
            role = $_.role
            stdoutSha256 = $_.stdout.sha256
            stderrSha256 = $_.stderr.sha256
            expectedExit = $_.expectedExit
        }
    })
} | ConvertTo-Json -Depth 5
