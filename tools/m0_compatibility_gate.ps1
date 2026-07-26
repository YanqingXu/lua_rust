<#
.SYNOPSIS
    Runs the M0 process-level compatibility gate.
.DESCRIPTION
    Builds and validates the pinned C++ and Lua 5.1.5 oracles, builds the
    Rust command-line tools, validates the complete Lua fixture manifest,
    runs the focused dual-oracle differential suite, and emits the
    non-official compatibility debt report.

    Ordinary differences in the 101-case non-official corpus are recorded as
    migration debt. Missing tools, invalid manifests, runner errors, timeouts,
    and focused four-case differential failures are hard gate failures.
.PARAMETER ReuseExistingOracles
    Reuse executables under target/oracles instead of rebuilding them. This is
    a smoke-only shortcut.
.PARAMETER SkipRustBuild
    Reuse the configured Rust executables instead of building lua_app and
    lua_bytecode. This is a smoke-only shortcut.
.PARAMETER SkipRunnerQuality
    Skip format, Clippy, and test checks for tools/lua_fixture_runner. Intended
    only for a smoke rerun; CI must not pass this switch.
.PARAMETER Smoke
    Permit explicitly partial local checks. Smoke runs use a separate result
    channel and never report a full M0 gate pass.
#>

param(
    [string]$Root = "",
    [string]$CppRoot = "",
    [ValidateSet("Debug", "Release")]
    [string]$RustConfiguration = "Debug",
    [string]$CandidateLua = "",
    [string]$CandidateBytecode = "",
    [string]$CppLua = "",
    [string]$OfficialLua = "",
    [string]$ResultDirectory = "target/compatibility",
    [string]$ResultPath = "target/compatibility/m0-compatibility-gate.json",
    [switch]$ReuseExistingOracles,
    [switch]$SkipRustBuild,
    [switch]$SkipRunnerQuality,
    [switch]$Smoke
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
} else {
    $Root = (Resolve-Path -LiteralPath $Root).Path
}
if ($Smoke -and -not $PSBoundParameters.ContainsKey("ResultPath")) {
    $ResultPath = "target/compatibility/m0-compatibility-smoke.json"
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

function Get-Sha256IfPresent {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path) -or
        -not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return ""
    }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$resultRoot = Resolve-RootedPath $ResultDirectory
$gateResultPath = Resolve-RootedPath $ResultPath
if (-not (Test-Path -LiteralPath $resultRoot)) {
    New-Item -ItemType Directory -Path $resultRoot -Force | Out-Null
}
$gateResultParent = Split-Path -Parent $gateResultPath
if (-not (Test-Path -LiteralPath $gateResultParent)) {
    New-Item -ItemType Directory -Path $gateResultParent -Force | Out-Null
}

$steps = [System.Collections.Generic.List[object]]::new()
$hardFailures = [System.Collections.Generic.List[string]]::new()
$debts = [System.Collections.Generic.List[object]]::new()
$gateStart = [System.Diagnostics.Stopwatch]::StartNew()
$hostExecutable = (Get-Process -Id $PID).Path

function Add-Step {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [ValidateSet("passed", "failed", "debt", "skipped")]
        [string]$Status,
        [Nullable[int]]$ExitCode = $null,
        [long]$DurationMs = 0,
        [string]$Detail = "",
        [string]$Artifact = ""
    )

    $steps.Add([ordered]@{
        name = $Name
        status = $Status
        exitCode = $ExitCode
        durationMs = $DurationMs
        detail = $Detail
        artifact = $Artifact
    })
    if ($Status -eq "failed") {
        $hardFailures.Add("${Name}: $Detail")
    }
}

function Invoke-ExternalStep {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [string]$FilePath,
        [string[]]$Arguments = @(),
        [int[]]$DebtExitCodes = @(),
        [string]$Artifact = ""
    )

    Write-Host "`n=== $Name ===" -ForegroundColor Cyan
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $exitCode = 127
    $detail = ""
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $looksLikePath = [System.IO.Path]::IsPathRooted($FilePath) -or
            $FilePath.Contains([System.IO.Path]::DirectorySeparatorChar) -or
            $FilePath.Contains([System.IO.Path]::AltDirectorySeparatorChar)
        if ($looksLikePath) {
            if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
                throw "required command does not exist: $FilePath"
            }
        } elseif ($null -eq (
            Get-Command $FilePath -ErrorAction SilentlyContinue |
                Select-Object -First 1
        )) {
            throw "required command is not available on PATH: $FilePath"
        }

        # Native tools routinely use stderr for progress. Under PowerShell 7,
        # ErrorActionPreference=Stop can otherwise turn a successful native
        # invocation into a terminating ErrorRecord before LASTEXITCODE is read.
        $ErrorActionPreference = "Continue"
        $output = @(& $FilePath @Arguments 2>&1)
        $exitCode = $LASTEXITCODE
        $output | ForEach-Object { Write-Host "$_" }
        if ($exitCode -ne 0 -and $DebtExitCodes -notcontains $exitCode) {
            $detail = "command exited with status $exitCode"
        } elseif ($DebtExitCodes -contains $exitCode) {
            $detail = "command reported migration debt with status $exitCode"
        }
    } catch {
        $detail = $_.Exception.Message
        Write-Host "[ERROR] $detail" -ForegroundColor Red
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
        $stopwatch.Stop()
    }

    $status = if ($exitCode -eq 0) {
        "passed"
    } elseif ($DebtExitCodes -contains $exitCode) {
        "debt"
    } else {
        "failed"
    }
    Add-Step `
        -Name $Name `
        -Status $status `
        -ExitCode $exitCode `
        -DurationMs $stopwatch.ElapsedMilliseconds `
        -Detail $detail `
        -Artifact $Artifact
    return $exitCode
}

function Invoke-PowerShellStep {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [string]$ScriptPath,
        [string[]]$Arguments = @(),
        [string]$Artifact = ""
    )

    $childArguments = @(
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        $ScriptPath
    ) + $Arguments
    return Invoke-ExternalStep `
        -Name $Name `
        -FilePath $hostExecutable `
        -Arguments $childArguments `
        -Artifact $Artifact
}

function Add-ArtifactValidation {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [string]$Artifact,
        [Parameter(Mandatory = $true)]
        [scriptblock]$Validate
    )

    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $status = "passed"
    $detail = ""
    try {
        if (-not (Test-Path -LiteralPath $Artifact -PathType Leaf)) {
            throw "required artifact was not produced: $Artifact"
        }
        $document = Get-Content -LiteralPath $Artifact -Raw | ConvertFrom-Json
        & $Validate $document
    } catch {
        $status = "failed"
        $detail = $_.Exception.Message
    } finally {
        $stopwatch.Stop()
    }
    Add-Step `
        -Name $Name `
        -Status $status `
        -DurationMs $stopwatch.ElapsedMilliseconds `
        -Detail $detail `
        -Artifact $Artifact
}

$requestedCoreSkips = [System.Collections.Generic.List[string]]::new()
if ($ReuseExistingOracles) {
    $requestedCoreSkips.Add("oracle-build")
}
if ($SkipRustBuild) {
    $requestedCoreSkips.Add("rust-build")
}
if ($SkipRunnerQuality) {
    $requestedCoreSkips.Add("fixture-runner-quality")
}
if ($requestedCoreSkips.Count -gt 0 -and -not $Smoke) {
    Add-Step `
        -Name "gate-mode-policy" `
        -Status "failed" `
        -ExitCode 2 `
        -Detail (
            "Core skips require explicit -Smoke mode: " +
            ($requestedCoreSkips -join ", ")
        )
} else {
    Add-Step `
        -Name "gate-mode-policy" `
        -Status "passed" `
        -ExitCode 0 `
        -Detail $(if ($Smoke) {
            "explicit smoke mode; this run cannot pass the M0 compatibility gate"
        } else {
            "full compatibility mode"
        })
}

if ([string]::IsNullOrWhiteSpace($CppRoot)) {
    if (-not [string]::IsNullOrWhiteSpace($env:LUA_CPP_ORACLE_ROOT)) {
        $CppRoot = $env:LUA_CPP_ORACLE_ROOT
    } else {
        $CppRoot = Join-Path (Split-Path -Parent $Root) "lua_cpp"
    }
}
$CppRoot = Resolve-RootedPath $CppRoot

$runningOnWindows = $env:OS -eq "Windows_NT"
$executableSuffix = if ($runningOnWindows) { ".exe" } else { "" }
$rustProfile = if ($RustConfiguration -eq "Release") { "release" } else { "debug" }
$rustTargetTriple = ""
try {
    $rustTargetTriple = (& rustc -vV 2>$null |
        Where-Object { $_ -match '^host:\s+(.+)$' } |
        ForEach-Object { $Matches[1] } |
        Select-Object -First 1)
} catch {
    # A missing Rust toolchain is reported by the build/artifact steps below,
    # after the gate has initialized its always-written JSON result.
}
if ([string]::IsNullOrWhiteSpace($rustTargetTriple)) {
    $rustTargetTriple = if ($runningOnWindows) {
        "x86_64-pc-windows-msvc"
    } else {
        "unknown-rust-target"
    }
}
$rustExecutableRoot = Join-Path $Root "target/$rustTargetTriple/$rustProfile"
if ([string]::IsNullOrWhiteSpace($CandidateLua)) {
    $CandidateLua = Join-Path $rustExecutableRoot "lua_app$executableSuffix"
} else {
    $CandidateLua = Resolve-RootedPath $CandidateLua
}
if ([string]::IsNullOrWhiteSpace($CandidateBytecode)) {
    $CandidateBytecode = Join-Path $rustExecutableRoot "lua_bytecode$executableSuffix"
} else {
    $CandidateBytecode = Resolve-RootedPath $CandidateBytecode
}

$cppOracleOutput = Join-Path $Root "target/oracles/lua_cpp"
$cppExecutableRoot = if ($runningOnWindows) {
    Join-Path $cppOracleOutput "build/Release"
} else {
    Join-Path $cppOracleOutput "build"
}
if ([string]::IsNullOrWhiteSpace($CppLua)) {
    $CppLua = Join-Path $cppExecutableRoot "lua_app$executableSuffix"
} else {
    $CppLua = Resolve-RootedPath $CppLua
}
$cppBytecode = Join-Path $cppExecutableRoot "lua_bytecode$executableSuffix"

$lua51OracleOutput = Join-Path $Root "target/oracles/lua-5.1.5"
if ([string]::IsNullOrWhiteSpace($OfficialLua)) {
    $OfficialLua = if ($runningOnWindows) {
        Join-Path $lua51OracleOutput "build/Release/lua51.exe"
    } else {
        Join-Path $lua51OracleOutput "build/lua51"
    }
} else {
    $OfficialLua = Resolve-RootedPath $OfficialLua
}

$baselineArtifact = Join-Path $resultRoot "oracle-baseline.json"
$cppBuildArtifact = Join-Path $resultRoot "cpp-oracle-build.json"
$lua51BuildArtifact = Join-Path $resultRoot "lua51-oracle-build.json"
$differentialArtifact = Join-Path $resultRoot "lua51-differential.json"
$nonOfficialArtifact = Join-Path $resultRoot "non-official.json"
$paritySelfTestRoot = Join-Path $resultRoot "parity-runner-self-test"
$paritySelfTestArtifact = Join-Path $resultRoot "parity-runner-self-test.json"
$bytecodeSelfTestReport = Join-Path $paritySelfTestRoot "bytecode/report.json"
$bytecodeDirectorySelfTestReport = Join-Path $paritySelfTestRoot `
    "bytecode-directory-representative/report.json"
$vmTraceSelfTestReport = Join-Path $paritySelfTestRoot "vm-trace/report.json"
$representativeBytecodeRoot = Join-Path $resultRoot "bytecode-representative"
$representativeBytecodeReport = Join-Path $representativeBytecodeRoot "report.json"
$runnerManifest = Join-Path $Root "tools/lua_fixture_runner/Cargo.toml"
$fixtureManifest = Join-Path $Root "tests/compatibility/lua_fixtures.json"
$fixtureInventoryTotal = 0

$fatalError = ""
Push-Location $Root
try {
    $fixtureInventoryTotal = [int](
        Get-Content -LiteralPath $fixtureManifest -Raw | ConvertFrom-Json
    ).inventory.current_total
    if ($fixtureInventoryTotal -le 0) {
        throw "fixture manifest inventory.current_total must be positive"
    }

    $null = Invoke-PowerShellStep `
        -Name "oracle-baseline" `
        -ScriptPath (Join-Path $Root "tools/check_oracle_baseline.ps1") `
        -Arguments @(
            "-Root", $Root,
            "-CppRoot", $CppRoot,
            "-ResultPath", $baselineArtifact
        ) `
        -Artifact $baselineArtifact

    if ($SkipRunnerQuality) {
        Add-Step `
            -Name "fixture-runner-quality" `
            -Status "skipped" `
            -Detail "explicitly skipped for a local fast rerun"
    } else {
        $null = Invoke-ExternalStep `
            -Name "fixture-runner-format" `
            -FilePath "cargo" `
            -Arguments @(
                "fmt",
                "--manifest-path", $runnerManifest,
                "--",
                "--check"
            )
        $null = Invoke-ExternalStep `
            -Name "fixture-runner-clippy" `
            -FilePath "cargo" `
            -Arguments @(
                "clippy",
                "--manifest-path", $runnerManifest,
                "--all-targets",
                "--",
                "-D", "warnings"
            )
        $null = Invoke-ExternalStep `
            -Name "fixture-runner-tests" `
            -FilePath "cargo" `
            -Arguments @(
                "test",
                "--manifest-path", $runnerManifest
            )
    }

    if ($SkipRustBuild) {
        Add-Step `
            -Name "rust-cli-build" `
            -Status "skipped" `
            -Detail "reusing existing Rust command-line executables"
    } else {
        $rustBuildArguments = @(
            "build",
            "--package", "lua_app",
            "--package", "lua_bytecode"
        )
        if ($RustConfiguration -eq "Release") {
            $rustBuildArguments += "--release"
        }
        $null = Invoke-ExternalStep `
            -Name "rust-cli-build" `
            -FilePath "cargo" `
            -Arguments $rustBuildArguments
    }

    $missingRustTools = @(@($CandidateLua, $CandidateBytecode) |
        Where-Object { -not (Test-Path -LiteralPath $_ -PathType Leaf) })
    if ($missingRustTools.Count -gt 0) {
        Add-Step `
            -Name "rust-cli-artifacts" `
            -Status "failed" `
            -Detail ("missing executable(s): " + ($missingRustTools -join ", "))
    } else {
        Add-Step `
            -Name "rust-cli-artifacts" `
            -Status "passed" `
            -Detail "lua_app and lua_bytecode are present"
    }

    if ($ReuseExistingOracles) {
        Add-Step `
            -Name "cpp-oracle-build" `
            -Status "skipped" `
            -Detail "reusing existing pinned C++ oracle executables" `
            -Artifact $cppBuildArtifact
        Add-Step `
            -Name "lua51-oracle-build" `
            -Status "skipped" `
            -Detail "reusing existing SHA-verified Lua 5.1.5 executable" `
            -Artifact $lua51BuildArtifact
    } else {
        $null = Invoke-PowerShellStep `
            -Name "cpp-oracle-build" `
            -ScriptPath (Join-Path $Root "tools/build_cpp_oracle.ps1") `
            -Arguments @(
                "-Root", $Root,
                "-CppRoot", $CppRoot,
                "-OutputDirectory", $cppOracleOutput,
                "-Configuration", "Release",
                "-ResultPath", $cppBuildArtifact
            ) `
            -Artifact $cppBuildArtifact
        $null = Invoke-PowerShellStep `
            -Name "lua51-oracle-build" `
            -ScriptPath (Join-Path $Root "tools/build_lua51_oracle.ps1") `
            -Arguments @(
                "-Root", $Root,
                "-OutputDirectory", $lua51OracleOutput,
                "-ResultPath", $lua51BuildArtifact
            ) `
            -Artifact $lua51BuildArtifact
    }

    $missingOracleTools = @(@($CppLua, $cppBytecode, $OfficialLua) |
        Where-Object { -not (Test-Path -LiteralPath $_ -PathType Leaf) })
    if ($missingOracleTools.Count -gt 0) {
        Add-Step `
            -Name "oracle-artifacts" `
            -Status "failed" `
            -Detail ("missing executable(s): " + ($missingOracleTools -join ", "))
    } else {
        Add-Step `
            -Name "oracle-artifacts" `
            -Status "passed" `
            -Detail "lua_cpp app/bytecode and official Lua 5.1.5 are present"
    }

    $paritySelfTestExit = Invoke-PowerShellStep `
        -Name "parity-runner-fail-closed-self-test" `
        -ScriptPath (Join-Path $Root "tools/test_parity_runners.ps1") `
        -Arguments @(
            "-InputPath", (Join-Path $Root "tests/lua/bytecode/test_bytecode.lua"),
            "-CppBytecodeExe", $cppBytecode,
            "-OutputDir", $paritySelfTestRoot
        ) `
        -Artifact $paritySelfTestArtifact

    $parityValidationStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $parityValidationStatus = "passed"
    $parityValidationDetail = ""
    $paritySummary = [ordered]@{
        schemaVersion = 1
        channel = "parity-runner-self-test"
        passed = $false
        bytecodeReport = $bytecodeSelfTestReport
        bytecodeDirectoryReport = $bytecodeDirectorySelfTestReport
        vmTraceReport = $vmTraceSelfTestReport
        failure = $null
    }
    try {
        if ($paritySelfTestExit -ne 0) {
            throw "self-test command exited with status $paritySelfTestExit"
        }
        foreach ($reportItem in @(
            [ordered]@{
                name = "bytecode"
                path = $bytecodeSelfTestReport
            },
            [ordered]@{
                name = "bytecode-directory-representative"
                path = $bytecodeDirectorySelfTestReport
            },
            [ordered]@{
                name = "synthetic-vm-trace"
                path = $vmTraceSelfTestReport
            }
        )) {
            if (-not (Test-Path -LiteralPath $reportItem.path -PathType Leaf)) {
                throw "$($reportItem.name) self-test report is missing"
            }
            $report = Get-Content -LiteralPath $reportItem.path -Raw |
                ConvertFrom-Json
            if ($report.schemaVersion -ne 2 -or
                $report.purpose -ne "infrastructure-self-test" -or
                $report.status -ne "self-test-passed" -or
                $report.summary.selected -lt 1 -or
                $report.summary.failed -ne 0 -or
                $report.summary.infrastructureFailures -ne 0) {
                throw "$($reportItem.name) self-test did not produce clean fail-closed evidence"
            }
        }
        $paritySummary.passed = $true
    } catch {
        $parityValidationStatus = "failed"
        $parityValidationDetail = $_.Exception.Message
        $paritySummary.failure = $parityValidationDetail
    } finally {
        $parityValidationStopwatch.Stop()
        $paritySummaryJson = $paritySummary | ConvertTo-Json -Depth 8
        [System.IO.File]::WriteAllText(
            $paritySelfTestArtifact,
            $paritySummaryJson + [Environment]::NewLine,
            [System.Text.UTF8Encoding]::new($false)
        )
    }
    Add-Step `
        -Name "parity-runner-self-test-artifacts" `
        -Status $parityValidationStatus `
        -DurationMs $parityValidationStopwatch.ElapsedMilliseconds `
        -Detail $parityValidationDetail `
        -Artifact $paritySelfTestArtifact

    $bytecodeChildArguments = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $Root "tools/compare_bytecode.ps1"),
        "-InputPath", (Join-Path $Root "tests/lua/bytecode"),
        "-CorpusMode", "Representative",
        "-CppBytecodeExe", $cppBytecode,
        "-RustBytecodeExe", $CandidateBytecode,
        "-OutputDir", $representativeBytecodeRoot,
        "-ResultPath", $representativeBytecodeReport
    )
    $representativeBytecodeExit = Invoke-ExternalStep `
        -Name "representative-bytecode-parity" `
        -FilePath $hostExecutable `
        -Arguments $bytecodeChildArguments `
        -DebtExitCodes @(1) `
        -Artifact $representativeBytecodeReport
    if ($representativeBytecodeExit -eq 1) {
        $debts.Add([ordered]@{
            id = "representative-bytecode-differences"
            acceptedBy = "M0.6 parity migration policy"
            artifact = $representativeBytecodeReport
            reason = "Structured bytecode evidence is runnable, but semantic differences remain"
        })
    }
    Add-ArtifactValidation `
        -Name "representative-bytecode-artifact" `
        -Artifact $representativeBytecodeReport `
        -Validate {
            param($document)

            if ($document.schemaVersion -ne 2 -or
                $document.purpose -ne "cross-language-parity") {
                throw "unsupported representative bytecode report"
            }
            if ($document.summary.selected -lt 1) {
                throw "representative bytecode corpus selected no inputs"
            }
            if ($document.summary.infrastructureFailures -ne 0) {
                throw "representative bytecode report contains infrastructure failures"
            }
            if ($document.status -notin @("passed", "differences-found")) {
                throw "unexpected representative bytecode status: $($document.status)"
            }
            foreach ($case in @($document.cases)) {
                foreach ($execution in @(
                    $case.executions.left,
                    $case.executions.right
                )) {
                    if ($execution.timedOut -or
                        $null -ne $execution.startError -or
                        $execution.exitCode -ne 0) {
                        throw "representative bytecode case lacks clean process evidence"
                    }
                }
            }
        }

    $debts.Add([ordered]@{
        id = "real-vm-trace-parity-unsupported"
        acceptedBy = "M0.6 parity migration policy"
        artifact = ""
        reason = "Synthetic fail-closed runner evidence passes; real lua_cpp/lua_rust --trace-diff support remains a local parity task"
        localCommand = "powershell -File tools/compare_vm_trace.ps1 -InputPath tests/lua -CorpusMode Representative"
    })

    $null = Invoke-ExternalStep `
        -Name "fixture-manifest-complete" `
        -FilePath "cargo" `
        -Arguments @(
            "run",
            "--quiet",
            "--manifest-path", $runnerManifest,
            "--",
            "--repo-root", $Root,
            "--manifest", $fixtureManifest,
            "--suite", "all",
            "--validate-only"
        )

    $null = Invoke-PowerShellStep `
        -Name "focused-dual-oracle-differential" `
        -ScriptPath (Join-Path $Root "tools/run_lua51_differential.ps1") `
        -Arguments @(
            "-Root", $Root,
            "-CandidateLua", $CandidateLua,
            "-OfficialLua", $OfficialLua,
            "-CppLua", $CppLua,
            "-CppRoot", $CppRoot,
            "-ResultPath", $differentialArtifact
        ) `
        -Artifact $differentialArtifact

    Add-ArtifactValidation `
        -Name "focused-differential-artifact" `
        -Artifact $differentialArtifact `
        -Validate {
            param($document)

            if ($document.schemaVersion -ne 1) {
                throw "unsupported differential artifact schema"
            }
            if (@($document.lanes).Count -ne 2) {
                throw "expected two differential lanes"
            }
            $laneIds = @($document.lanes | ForEach-Object id)
            foreach ($requiredLane in @("official-lua51", "cpp-87c15e6")) {
                if ($laneIds -notcontains $requiredLane) {
                    throw "missing differential lane: $requiredLane"
                }
            }
            foreach ($lane in @($document.lanes)) {
                if (@($lane.cases).Count -ne 4) {
                    throw "lane $($lane.id) did not execute exactly four cases"
                }
                $observations = @($lane.versionProbe.reference, $lane.versionProbe.candidate)
                foreach ($case in @($lane.cases)) {
                    $observations += @($case.reference, $case.candidate)
                }
                $nonCompleted = @($observations |
                    Where-Object { $_.outcome -ne "completed" })
                if ($nonCompleted.Count -gt 0) {
                    throw "lane $($lane.id) contains timeout or infrastructure outcomes"
                }
            }
            if (-not $document.passed) {
                throw "focused differential semantics did not pass"
            }
        }

    $null = Invoke-ExternalStep `
        -Name "non-official-101-run" `
        -FilePath "cargo" `
        -Arguments @(
            "run",
            "--quiet",
            "--manifest-path", $runnerManifest,
            "--",
            "--repo-root", $Root,
            "--manifest", $fixtureManifest,
            "--rust", $CandidateLua,
            "--cpp", $CppLua,
            "--artifact", $nonOfficialArtifact,
            "--suite", "non-official",
            "--allow-differences"
        ) `
        -Artifact $nonOfficialArtifact

    $nonOfficialStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $nonOfficialStatus = "passed"
    $nonOfficialDetail = ""
    try {
        if (-not (Test-Path -LiteralPath $nonOfficialArtifact -PathType Leaf)) {
            throw "required artifact was not produced: $nonOfficialArtifact"
        }
        $nonOfficial = Get-Content -LiteralPath $nonOfficialArtifact -Raw |
            ConvertFrom-Json
        if ($nonOfficial.schema_version -ne "lua-fixture-differential/v1") {
            throw "unsupported non-official artifact schema"
        }
        if ($nonOfficial.fixture_inventory.current_total -ne $fixtureInventoryTotal) {
            throw (
                "artifact fixture total $($nonOfficial.fixture_inventory.current_total) " +
                "does not match manifest total $fixtureInventoryTotal"
            )
        }
        if ($nonOfficial.summary.selected -ne 101 -or
            @($nonOfficial.cases).Count -ne 101) {
            throw "non-official suite must contain exactly 101 records"
        }
        if ($nonOfficial.summary.runner_errors -ne 0) {
            throw "non-official runner reported $($nonOfficial.summary.runner_errors) errors"
        }
        if ($nonOfficial.summary.timed_out -ne 0) {
            throw "non-official runner reported $($nonOfficial.summary.timed_out) timeouts"
        }
        if ($nonOfficial.summary.executed +
            $nonOfficial.summary.helpers_skipped -ne 101) {
            throw "non-official execution accounting does not total 101"
        }
        if ($nonOfficial.summary.differences -gt 0) {
            $nonOfficialStatus = "debt"
            $nonOfficialDetail = (
                "{0} known semantic differences; {1} matches; {2} helpers skipped" -f
                $nonOfficial.summary.differences,
                $nonOfficial.summary.matches,
                $nonOfficial.summary.helpers_skipped
            )
            $debts.Add([ordered]@{
                id = "non-official-semantic-differences"
                acceptedBy = "M0 compatibility policy"
                artifact = $nonOfficialArtifact
                differences = $nonOfficial.summary.differences
                matches = $nonOfficial.summary.matches
                helpersSkipped = $nonOfficial.summary.helpers_skipped
            })
        }
    } catch {
        $nonOfficialStatus = "failed"
        $nonOfficialDetail = $_.Exception.Message
    } finally {
        $nonOfficialStopwatch.Stop()
    }
    Add-Step `
        -Name "non-official-artifact-policy" `
        -Status $nonOfficialStatus `
        -DurationMs $nonOfficialStopwatch.ElapsedMilliseconds `
        -Detail $nonOfficialDetail `
        -Artifact $nonOfficialArtifact
} catch {
    $fatalError = $_.Exception.Message
    Add-Step `
        -Name "compatibility-gate-internal" `
        -Status "failed" `
        -Detail $fatalError
} finally {
    Pop-Location
    $gateStart.Stop()
    $checksPassed = $hardFailures.Count -eq 0
    $fullGatePassed = (
        $checksPassed -and
        -not $Smoke -and
        -not $ReuseExistingOracles -and
        -not $SkipRustBuild -and
        -not $SkipRunnerQuality
    )
    $document = [ordered]@{
        schemaVersion = 1
        channel = if ($Smoke) {
            "m0-compatibility-smoke"
        } else {
            "m0-compatibility-gate"
        }
        mode = if ($Smoke) { "smoke" } else { "full" }
        generatedAtUtc = [DateTime]::UtcNow.ToString("o")
        passed = $fullGatePassed
        checksPassed = $checksPassed
        fullGatePassed = $fullGatePassed
        durationMs = $gateStart.ElapsedMilliseconds
        policy = [ordered]@{
            focusedDifferentialMustMatch = $true
            nonOfficialSemanticDifferencesAreDebt = $true
            manifestRunnerErrorsAndTimeoutsFail = $true
        }
        inputs = [ordered]@{
            root = $Root
            cppRoot = $CppRoot
            rustConfiguration = $RustConfiguration
            reusedExistingOracles = [bool]$ReuseExistingOracles
            skippedRustBuild = [bool]$SkipRustBuild
            skippedRunnerQuality = [bool]$SkipRunnerQuality
            smoke = [bool]$Smoke
        }
        executables = [ordered]@{
            rustLua = [ordered]@{
                path = $CandidateLua
                sha256 = Get-Sha256IfPresent $CandidateLua
            }
            rustBytecode = [ordered]@{
                path = $CandidateBytecode
                sha256 = Get-Sha256IfPresent $CandidateBytecode
            }
            cppLua = [ordered]@{
                path = $CppLua
                sha256 = Get-Sha256IfPresent $CppLua
            }
            cppBytecode = [ordered]@{
                path = $cppBytecode
                sha256 = Get-Sha256IfPresent $cppBytecode
            }
            officialLua51 = [ordered]@{
                path = $OfficialLua
                sha256 = Get-Sha256IfPresent $OfficialLua
            }
        }
        steps = @($steps)
        debts = @($debts)
        hardFailures = @($hardFailures)
        fatalError = $fatalError
    }
    $json = $document | ConvertTo-Json -Depth 12
    [System.IO.File]::WriteAllText(
        $gateResultPath,
        $json + [Environment]::NewLine,
        [System.Text.UTF8Encoding]::new($false)
    )
    Write-Host "`n[INFO] M0 compatibility gate report: $gateResultPath"
}

if ($hardFailures.Count -gt 0) {
    Write-Host "[FAIL] M0 compatibility gate failed:" -ForegroundColor Red
    $hardFailures | ForEach-Object { Write-Host " - $_" }
    exit 1
}

if ($Smoke) {
    Write-Host (
        "[OK] M0 smoke checks passed; this is not a full compatibility-gate pass"
    )
} elseif ($debts.Count -gt 0) {
    Write-Host "[OK] M0 infrastructure passed; known semantic debt was reported"
} else {
    Write-Host "[OK] M0 compatibility gate passed without recorded debt"
}
