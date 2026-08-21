param(
    [string]$CandidateLua = "",
    [Parameter(Mandatory = $true)]
    [string]$OfficialLua,
    [Parameter(Mandatory = $true)]
    [string]$CppLua,
    [Parameter(Mandatory = $true)]
    [string]$CppRoot,
    [string]$CppArchive = "",
    [string]$ResultPath = "target/compatibility/lua51-differential.json",
    [ValidateRange(1, 120)]
    [int]$TimeoutSeconds = 10
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$snapshotRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$expectedCppCommit = '87c15e69ceb94eb74e28226ccbefb7e196635711'
$expectedCppArchiveSha256 = '66c9506a07c4bab8b8e84d7858484e1456772bf605be2ae40003a22a8d1bbfb8'
$hostLine = (& rustc -vV | Select-String -Pattern '^host:').Line
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($hostLine)) {
    throw 'Could not determine the native Rust host triple'
}
$hostTriple = ($hostLine -split ':', 2)[1].Trim()

function Resolve-File([string]$Path, [string]$Role) {
    $resolved = if ([IO.Path]::IsPathRooted($Path)) {
        [IO.Path]::GetFullPath($Path)
    } else {
        [IO.Path]::GetFullPath((Join-Path $snapshotRoot $Path))
    }
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "Missing ${Role}: $resolved"
    }
    return $resolved
}

function Invoke-Lua([string]$Executable, [string]$Script) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Executable
    $start.WorkingDirectory = $snapshotRoot
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.ArgumentList.Add($Script)
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    if (-not $process.Start()) {
        throw "Could not start $Executable"
    }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        $process.Kill($true)
        $process.WaitForExit()
        throw "Timed out after $TimeoutSeconds seconds: $Executable $Script"
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult().Replace("`r`n", "`n")
    $stderr = $stderrTask.GetAwaiter().GetResult().Replace("`r`n", "`n")
    return [ordered]@{
        exitCode = $process.ExitCode
        stdout = $stdout
        stderr = $stderr
    }
}

$official = Resolve-File $OfficialLua 'official Lua 5.1.5 executable'
$cpp = Resolve-File $CppLua 'fixed C++ oracle executable'
$cppCheckout = (Resolve-Path -LiteralPath $CppRoot).Path
$gitDirectory = Join-Path $cppCheckout '.git'
if (Test-Path -LiteralPath $gitDirectory) {
    $actualCommit = (& git -C $cppCheckout rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $actualCommit -ne $expectedCppCommit) {
        throw "C++ oracle commit mismatch: expected $expectedCppCommit, got $actualCommit"
    }
    $cppProvenance = "git:$actualCommit"
} else {
    if ([string]::IsNullOrWhiteSpace($CppArchive)) {
        throw 'Non-git C++ source requires -CppArchive for SHA-256 provenance verification'
    }
    $archive = Resolve-File $CppArchive 'fixed C++ oracle source archive'
    $archiveHash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    $expectedDirectoryName = "lua-$expectedCppCommit"
    if ($archiveHash -ne $expectedCppArchiveSha256 -or
        (Split-Path -Leaf $cppCheckout) -ne $expectedDirectoryName -or
        -not (Test-Path -LiteralPath (Join-Path $cppCheckout 'CMakeLists.txt') -PathType Leaf)) {
        throw "C++ archive provenance mismatch: expected commit $expectedCppCommit and SHA-256 $expectedCppArchiveSha256"
    }
    $actualCommit = $expectedCppCommit
    $cppProvenance = "archive-sha256:$archiveHash"
}

if ([string]::IsNullOrWhiteSpace($CandidateLua)) {
    & cargo build --quiet --manifest-path (Join-Path $snapshotRoot 'Cargo.toml') `
        --target $hostTriple
    if ($LASTEXITCODE -ne 0) {
        throw 'Candidate build failed'
    }
    $binary = if ($IsWindows) { 'lua_app.exe' } else { 'lua_app' }
    $metadata = (& cargo metadata --format-version 1 --no-deps `
        --manifest-path (Join-Path $snapshotRoot 'Cargo.toml') | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0) {
        throw 'Could not locate the candidate Cargo target directory'
    }
    $candidateBinaries = @(Get-ChildItem -LiteralPath $metadata.target_directory `
        -Recurse -File -Filter $binary | Where-Object { $_.Directory.Name -eq 'debug' })
    if ($candidateBinaries.Count -ne 1) {
        throw "Expected one debug candidate binary, found $($candidateBinaries.Count)"
    }
    $CandidateLua = $candidateBinaries[0].FullName
}
$candidate = Resolve-File $CandidateLua 'v1.0 candidate executable'

$manifest = Join-Path $snapshotRoot 'compatibility/differential-cases.tsv'
$cases = Import-Csv -LiteralPath $manifest -Delimiter "`t"
$results = [Collections.Generic.List[object]]::new()
$passed = $true
foreach ($case in $cases) {
    $script = Resolve-File $case.script "fixture '$($case.id)'"
    $observations = [ordered]@{
        candidate = Invoke-Lua $candidate $script
        official = Invoke-Lua $official $script
        cpp = Invoke-Lua $cpp $script
    }
    $casePassed =
        $observations.candidate.exitCode -eq $observations.official.exitCode -and
        $observations.candidate.exitCode -eq $observations.cpp.exitCode -and
        $observations.candidate.stdout -ceq $observations.official.stdout -and
        $observations.candidate.stdout -ceq $observations.cpp.stdout -and
        $observations.candidate.stderr -ceq $observations.official.stderr -and
        $observations.candidate.stderr -ceq $observations.cpp.stderr
    $passed = $passed -and $casePassed
    $results.Add([ordered]@{
        id = $case.id
        passed = $casePassed
        observations = $observations
    })
}

$report = [ordered]@{
    schema = 'lua-rust-lua51-differential/v1'
    passed = $passed
    cppCommit = $actualCommit
    cppProvenance = $cppProvenance
    candidate = $candidate
    official = $official
    cpp = $cpp
    results = $results
}
$output = if ([IO.Path]::IsPathRooted($ResultPath)) {
    [IO.Path]::GetFullPath($ResultPath)
} else {
    [IO.Path]::GetFullPath((Join-Path $snapshotRoot $ResultPath))
}
$parent = Split-Path -Parent $output
[IO.Directory]::CreateDirectory($parent) | Out-Null
[IO.File]::WriteAllText($output, ($report | ConvertTo-Json -Depth 8))
if (-not $passed) {
    throw "Differential mismatch; see $output"
}
Write-Host "All $($results.Count) fixtures matched both pinned oracles."
Write-Host "Report: $output"
