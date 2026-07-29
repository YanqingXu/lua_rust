<#
.SYNOPSIS
    Validates the implemented M1 foundation slice.
.DESCRIPTION
    Runs fail-closed static guards, the GC root-inventory validator, the raw
    byte comparator self-test, the Rust quality gate, and the focused M1
    byte differential suite. Passing this script is evidence for the current
    ByteString/ownership foundation only; it never declares all of M1 done.
.PARAMETER Smoke
    Permit explicit local skips. Smoke runs use a separate result channel and
    never report a full M1 foundation pass.
.PARAMETER SkipAudit
    Skip cargo-audit only in Smoke mode.
#>

param(
    [string]$Root = "",
    [string]$CppRoot = "",
    [string]$CandidateLua = "",
    [string]$CppLua = "",
    [string]$OfficialLua = "",
    [string]$ResultPath = "target/compatibility/m1-foundation-gate.json",
    [switch]$SkipQualityGate,
    [switch]$SkipDifferential,
    [switch]$SkipAudit,
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
    $ResultPath = "target/compatibility/m1-foundation-smoke.json"
}

function Resolve-RootedPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $Root $Path))
}

$steps = [System.Collections.Generic.List[object]]::new()
$failures = [System.Collections.Generic.List[string]]::new()
$gateTimer = [System.Diagnostics.Stopwatch]::StartNew()
$powershell = (Get-Process -Id $PID).Path

function Add-Step {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [ValidateSet("passed", "failed", "skipped")]
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
        $failures.Add("${Name}: $Detail")
    }
}

function Invoke-Step {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [string]$Executable,
        [string[]]$Arguments = @(),
        [string]$Artifact = ""
    )

    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $exitCode = 127
    $detail = ""
    $oldPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = @(& $Executable @Arguments 2>&1)
        $exitCode = $LASTEXITCODE
        if ($exitCode -ne 0) {
            $detail = (@($output | Select-Object -Last 40) -join "`n")
            if ([string]::IsNullOrWhiteSpace($detail)) {
                $detail = "command exited with status $exitCode"
            }
        }
    } catch {
        $detail = $_.Exception.Message
    } finally {
        $ErrorActionPreference = $oldPreference
        $timer.Stop()
    }

    Add-Step `
        -Name $Name `
        -Status $(if ($exitCode -eq 0) { "passed" } else { "failed" }) `
        -ExitCode $exitCode `
        -DurationMs $timer.ElapsedMilliseconds `
        -Detail $detail `
        -Artifact $Artifact
}

function Invoke-PowerShellStep {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [string]$Script,
        [string[]]$Arguments = @(),
        [string]$Artifact = ""
    )

    Invoke-Step `
        -Name $Name `
        -Executable $powershell `
        -Arguments (@(
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            $Script
        ) + $Arguments) `
        -Artifact $Artifact
}

function Test-RequiredFiles {
    $required = @(
        "crates/lua_core/src/byte_string.rs",
        "crates/lua_core/src/state_handle.rs",
        "crates/lua_vm/src/runtime.rs",
        "crates/lua_vm/src/runtime/root_trace.rs",
        "docs/rust_migration/byte_string_rfc.md",
        "docs/rust_migration/runtime_ownership_rfc.md",
        "tests/compatibility/gc_root_inventory.json",
        "tests/compatibility/m1-byte-differential-cases.json",
        "tools/check_gc_root_inventory.ps1",
        "tools/run_lua51_differential.ps1"
    )
    $missing = @($required | Where-Object {
        -not (Test-Path -LiteralPath (Join-Path $Root $_) -PathType Leaf)
    })
    if ($missing.Count -ne 0) {
        throw "Missing required M1 foundation files: $($missing -join ', ')"
    }
    if (Test-Path -LiteralPath (
        Join-Path $Root "crates/lua_stdlib/src/dump.rs"
    )) {
        throw "Legacy pseudo-dump registry source has returned"
    }
}

function Test-M1DifferentialArtifact {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "M1 differential artifact was not produced: $Path"
    }
    $document = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    if ($document.schemaVersion -ne 1 -or
        $document.channel -ne "lua51-differential" -or
        -not $document.passed) {
        throw "M1 differential artifact does not report a passing schema-v1 run"
    }
    if (@($document.lanes).Count -ne 2) {
        throw "M1 differential artifact must contain exactly two lanes"
    }
    $laneIds = @($document.lanes | ForEach-Object id)
    foreach ($requiredLane in @("official-lua51", "cpp-87c15e6")) {
        if ($laneIds -notcontains $requiredLane) {
            throw "M1 differential artifact lacks lane '$requiredLane'"
        }
    }
    foreach ($lane in @($document.lanes)) {
        if (@($lane.cases).Count -ne 2) {
            throw "M1 differential lane '$($lane.id)' must contain exactly two cases"
        }
        $caseIds = @($lane.cases | ForEach-Object id)
        foreach ($requiredCase in @(
            "m1-byte-string",
            "m1-byte-chunk-source"
        )) {
            if ($caseIds -notcontains $requiredCase) {
                throw (
                    "M1 differential lane '$($lane.id)' lacks case " +
                    "'$requiredCase'"
                )
            }
        }
        $observations = @(
            $lane.versionProbe.reference,
            $lane.versionProbe.candidate
        )
        foreach ($case in @($lane.cases)) {
            $observations += @($case.reference, $case.candidate)
        }
        if (@($observations | Where-Object {
            $_.outcome -ne "completed"
        }).Count -ne 0) {
            throw "M1 differential lane '$($lane.id)' contains a non-completed process"
        }
    }
    if (@($document.expectedDifferences).Count -ne 2) {
        throw "M1 differential artifact must account for two expected differences"
    }
    $approvedDeviationIds = @($document.deviationRegistry.approvedIds)
    foreach ($expected in @($document.expectedDifferences)) {
        if ($approvedDeviationIds -notcontains $expected.deviation) {
            throw (
                "Expected difference '$($expected.id)' is not backed by an " +
                "approved deviation registry entry"
            )
        }
        if ($expected.applicable -and
            -not $expected.optional -and
            $expected.consumed -ne 1) {
            throw (
                "Required expected difference '$($expected.id)' was consumed " +
                "$($expected.consumed) times"
            )
        }
    }
}

function Assert-NoMatches {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [string]$Pattern,
        [Parameter(Mandatory = $true)]
        [string[]]$Paths
    )

    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $oldPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $arguments = @("-n", "--pcre2", $Pattern) + $Paths
        $matches = @(& rg @arguments 2>&1)
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $oldPreference
        $timer.Stop()
    }

    if ($exitCode -eq 1) {
        Add-Step -Name $Name -Status "passed" -ExitCode 0 `
            -DurationMs $timer.ElapsedMilliseconds
        return
    }
    if ($exitCode -eq 0) {
        Add-Step -Name $Name -Status "failed" -ExitCode 1 `
            -DurationMs $timer.ElapsedMilliseconds `
            -Detail (@($matches | Select-Object -First 30) -join "`n")
        return
    }
    Add-Step -Name $Name -Status "failed" -ExitCode $exitCode `
        -DurationMs $timer.ElapsedMilliseconds `
        -Detail "rg failed: $(@($matches | Select-Object -Last 20) -join "`n")"
}

$requestedCoreSkips = [System.Collections.Generic.List[string]]::new()
if ($SkipQualityGate) {
    $requestedCoreSkips.Add("quality")
}
if ($SkipDifferential) {
    $requestedCoreSkips.Add("differential")
}
if ($SkipAudit) {
    $requestedCoreSkips.Add("audit")
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
            "explicit smoke mode; this run cannot pass the M1 foundation gate"
        } else {
            "full foundation mode"
        })
}

Push-Location $Root
try {
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        Test-RequiredFiles
        Add-Step -Name "required-foundation-files" -Status "passed" `
            -DurationMs $timer.ElapsedMilliseconds
    } catch {
        Add-Step -Name "required-foundation-files" -Status "failed" `
            -DurationMs $timer.ElapsedMilliseconds -Detail $_.Exception.Message
    }

    Assert-NoMatches `
        -Name "legacy-gcstring-api" `
        -Pattern "legacy_text|legacy_text_bytes|GcString::new\s*\(|GcString::compute_hash\s*\(" `
        -Paths @("crates")
    Assert-NoMatches `
        -Name "legacy-string-pool-api" `
        -Pattern "(pool|string_pool)\.(intern|find|for_each)\s*\(|pub\s+fn\s+(intern|find|for_each)\s*[<(]" `
        -Paths @("crates")
    Assert-NoMatches `
        -Name "compiler-service-raw-pointers" `
        -Pattern "\*mut\s+(GarbageCollector|StringPool)|unsafe\s+impl\s+(Send|Sync)\s+for\s+(CodeGenerator|BytecodeBuilder)" `
        -Paths @("crates/lua_compiler")
    Assert-NoMatches `
        -Name "production-direct-compiler-allocation" `
        -Pattern "CodeGenerator::new(?:_with_pool)?\s*\(|gc\.create\s*\(\s*(?:proto|Function::new_lua)" `
        -Paths @(
            "crates/lua_app/src/main.rs",
            "crates/lua_bytecode/src/main.rs",
            "crates/lua_stdlib/src/base.rs",
            "crates/lua_stdlib/src/package.rs"
        )
    Assert-NoMatches `
        -Name "library-package-direct-publication" `
        -Pattern "gc\.create\s*\(" `
        -Paths @(
            "crates/lua_stdlib/src/catalog.rs",
            "crates/lua_stdlib/src/package.rs"
        )
    Assert-NoMatches `
        -Name "legacy-library-registration-allocation" `
        -Pattern "let\s+(?:name_str|func_obj)\s*=\s*gc\.create" `
        -Paths @("crates/lua_stdlib/src")
    Assert-NoMatches `
        -Name "runtime-static-mut-state" `
        -Pattern "&'static\s+mut\s+LuaState" `
        -Paths @("crates")
    Assert-NoMatches `
        -Name "io-detached-static-references" `
        -Pattern "Option\s*<\s*&'static\s+(mut\s+)?(Table|IoFileData)" `
        -Paths @("crates/lua_stdlib/src/io.rs")
    Assert-NoMatches `
        -Name "io-direct-publication" `
        -Pattern "gc\.create\s*\(" `
        -Paths @("crates/lua_stdlib/src/io.rs")
    Assert-NoMatches `
        -Name "pseudo-dump-registry" `
        -Pattern "thread_local!\s*\{|(?:DUMPS|SOURCES)\s*:" `
        -Paths @("crates/lua_stdlib")

    $inventoryArtifact =
        Resolve-RootedPath "target/compatibility/gc-root-inventory.json"
    Invoke-PowerShellStep `
        -Name "gc-root-inventory" `
        -Script (Join-Path $Root "tools/check_gc_root_inventory.ps1") `
        -Arguments @("-Root", $Root, "-ResultPath", $inventoryArtifact) `
        -Artifact $inventoryArtifact

    Invoke-PowerShellStep `
        -Name "raw-byte-comparator-self-test" `
        -Script (Join-Path $Root "tools/run_lua51_differential.ps1") `
        -Arguments @("-Root", $Root, "-ComparatorSelfTestOnly")

    if ($SkipQualityGate) {
        Add-Step -Name "rust-quality-gate" -Status "skipped" `
            -Detail "explicitly skipped by caller"
    } else {
        $qualityArguments = @()
        if ($SkipAudit) {
            $qualityArguments += "-SkipAudit"
        }
        if ($Smoke) {
            $qualityArguments += "-Smoke"
        }
        Invoke-PowerShellStep `
            -Name "rust-quality-gate" `
            -Script (Join-Path $Root "tools/rust_quality_gate.ps1") `
            -Arguments $qualityArguments
    }

    if ($SkipDifferential) {
        Add-Step -Name "m1-byte-differential" -Status "skipped" `
            -Detail "explicitly skipped by caller"
    } else {
        if ([string]::IsNullOrWhiteSpace($CppRoot)) {
            $CppRoot = if (-not [string]::IsNullOrWhiteSpace(
                $env:LUA_CPP_ORACLE_ROOT
            )) {
                $env:LUA_CPP_ORACLE_ROOT
            } else {
                Join-Path (Split-Path -Parent $Root) "lua_cpp"
            }
        }
        if ([string]::IsNullOrWhiteSpace($OfficialLua)) {
            $OfficialLua = Join-Path $Root (
                "target/oracles/lua-5.1.5/build/Release/lua51.exe"
            )
        }
        if ([string]::IsNullOrWhiteSpace($CppLua)) {
            $CppLua = Join-Path $Root (
                "target/oracles/lua_cpp/build/Release/lua_app.exe"
            )
        }

        $candidateBuildPassed = $true
        if ([string]::IsNullOrWhiteSpace($CandidateLua)) {
            Invoke-Step `
                -Name "rust-candidate-build" `
                -Executable "cargo" `
                -Arguments @("build", "--package", "lua_app")
            $candidateBuildPassed =
                $steps[$steps.Count - 1].status -eq "passed"
        } else {
            Add-Step `
                -Name "rust-candidate-build" `
                -Status "skipped" `
                -Detail "caller supplied a self-managed CandidateLua executable"
        }

        $differentialArtifact =
            Resolve-RootedPath "target/compatibility/m1-byte-differential.json"
        if ($candidateBuildPassed) {
            $arguments = @(
                "-Root", $Root,
                "-CppRoot", $CppRoot,
                "-CppLua", $CppLua,
                "-OfficialLua", $OfficialLua,
                "-CasesPath", "tests/compatibility/m1-byte-differential-cases.json",
                "-ResultPath", $differentialArtifact
            )
            if (-not [string]::IsNullOrWhiteSpace($CandidateLua)) {
                $arguments += @("-CandidateLua", $CandidateLua)
            }
            Invoke-PowerShellStep `
                -Name "m1-byte-differential" `
                -Script (Join-Path $Root "tools/run_lua51_differential.ps1") `
                -Arguments $arguments `
                -Artifact $differentialArtifact
            if ($steps[$steps.Count - 1].status -eq "passed") {
                $artifactTimer =
                    [System.Diagnostics.Stopwatch]::StartNew()
                try {
                    Test-M1DifferentialArtifact -Path $differentialArtifact
                    Add-Step `
                        -Name "m1-byte-differential-artifact" `
                        -Status "passed" `
                        -DurationMs $artifactTimer.ElapsedMilliseconds `
                        -Artifact $differentialArtifact
                } catch {
                    Add-Step `
                        -Name "m1-byte-differential-artifact" `
                        -Status "failed" `
                        -DurationMs $artifactTimer.ElapsedMilliseconds `
                        -Detail $_.Exception.Message `
                        -Artifact $differentialArtifact
                } finally {
                    $artifactTimer.Stop()
                }
            } else {
                Add-Step `
                    -Name "m1-byte-differential-artifact" `
                    -Status "skipped" `
                    -Detail "differential command failed"
            }
        } else {
            Add-Step `
                -Name "m1-byte-differential" `
                -Status "skipped" `
                -Detail "candidate build failed; differential was not executed" `
                -Artifact $differentialArtifact
        }
    }
} catch {
    $failures.Add("gate infrastructure: $($_.Exception.Message)")
} finally {
    Pop-Location
    $gateTimer.Stop()
}

$openDebts = @(
    [ordered]@{
        id = "lua-state-service-backpointers"
        blocks = "M1 complete"
        detail = "LuaState still stores transitional raw GC/StringPool service pointers."
    },
    [ordered]@{
        id = "production-publication-roots"
        blocks = "destructive sweep"
        detail = "Active/debug Proto identities, open Upvalue owners, coroutine activation buffers, and PendingState handles are canonical roots; coroutine create/wrap, compiler Proto-to-Function, library/package, and IO object publication are transactional, while VM/app/result publication remains incomplete."
    },
    [ordered]@{
        id = "deterministic-runtime-shutdown"
        blocks = "M1.8"
        detail = "Deterministic state -> Thread -> ordinary -> fixed teardown and zero object/root/string/queue/state accounting are implemented; Lua-visible __gc drain, explicit service drain, and allocator live-byte proof remain open."
    },
    [ordered]@{
        id = "public-full-and-incremental-gc"
        blocks = "M1.9-M1.12"
        detail = "Lua-visible full sweep, complete barriers, weak/finalizer semantics, and incremental phases remain gated."
    },
    [ordered]@{
        id = "generational-gc-handles-and-publication-roots"
        blocks = "destructive sweep"
        detail = "GcRef carries non-reused ObjectId provenance, and StateHandle uses an opaque checked RuntimeId namespace plus MAX-generation slot retirement; lexical object roots plus coroutine, compiler, library/package, and IO publication are implemented, but VM/app/result graphs are not yet migrated."
    },
    [ordered]@{
        id = "string-content-equality-without-collector-borrow"
        blocks = "destructive sweep"
        detail = "Safe Value::String equality and hashing still preserve byte-content semantics through transitional unscoped dereferences; production string canonicalization/scoped access is not complete."
    }
)

$result = [ordered]@{
    schemaVersion = 1
    channel = if ($Smoke) {
        "m1-foundation-smoke"
    } else {
        "m1-foundation-gate"
    }
    mode = if ($Smoke) { "smoke" } else { "full" }
    scope = "ByteString, GcRef provenance, managed Proto, checked open-Upvalue and coroutine activation roots, temporary object/PendingState roots, compiler, library/package, and IO publication, fail-closed StateHandle identity/generation, Runtime/StateArena shutdown, and byte differential"
    checksPassed = $failures.Count -eq 0
    foundationPassed = (
        $failures.Count -eq 0 -and
        -not $Smoke -and
        -not $SkipQualityGate -and
        -not $SkipDifferential -and
        -not $SkipAudit
    )
    m1Complete = $false
    durationMs = $gateTimer.ElapsedMilliseconds
    smoke = [bool]$Smoke
    skippedQualityGate = [bool]$SkipQualityGate
    skippedDifferential = [bool]$SkipDifferential
    skippedAudit = [bool]$SkipAudit
    steps = @($steps)
    hardFailures = @($failures)
    openDebts = $openDebts
}

$resolvedResultPath = Resolve-RootedPath $ResultPath
$resultDirectory = Split-Path -Parent $resolvedResultPath
[System.IO.Directory]::CreateDirectory($resultDirectory) | Out-Null
$json = $result | ConvertTo-Json -Depth 10
Set-Content -LiteralPath $resolvedResultPath -Value $json -Encoding utf8
Write-Output $json

if ($failures.Count -ne 0) {
    exit 1
}
exit 0
