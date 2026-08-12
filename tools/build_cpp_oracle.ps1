param(
    [string]$Root = "",
    [string]$CppRoot = "",
    [string]$OutputDirectory = "target/oracles/lua_cpp",
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release",
    [string]$ResultPath = "target/compatibility/cpp-oracle-build.json"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$expectedCommit = "87c15e69ceb94eb74e28226ccbefb7e196635711"
$expectedRepository = "https://github.com/YanqingXu/lua.git"

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

function Normalize-GitUrl {
    param([string]$Url)
    return $Url.Trim().TrimEnd("/").ToLowerInvariant() -replace '\.git$', ''
}

function Write-Result {
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Document
    )
    $resolvedResultPath = Resolve-RootedPath $ResultPath
    $parent = Split-Path -Parent $resolvedResultPath
    if (-not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    $json = $Document | ConvertTo-Json -Depth 6
    [System.IO.File]::WriteAllText(
        $resolvedResultPath,
        $json + [Environment]::NewLine,
        [System.Text.UTF8Encoding]::new($false)
    )
    return $resolvedResultPath
}

if ([string]::IsNullOrWhiteSpace($CppRoot)) {
    if (-not [string]::IsNullOrWhiteSpace($env:LUA_CPP_ORACLE_ROOT)) {
        $CppRoot = $env:LUA_CPP_ORACLE_ROOT
    } else {
        $CppRoot = Join-Path (Split-Path -Parent $Root) "lua_cpp"
    }
}
$cppPath = Resolve-RootedPath $CppRoot
$outputRoot = Resolve-RootedPath $OutputDirectory
$buildDirectory = Join-Path $outputRoot "build"
$installDirectory = Join-Path $outputRoot "install"
$evidenceBuildDirectory = Join-Path $outputRoot "evidence-build"
$evidenceSourceDirectory = Join-Path $Root "tools/cpp_bytecode_evidence"
$traceEvidenceBuildDirectory = Join-Path $outputRoot "trace-evidence-build"
$traceEvidenceSourceDirectory = Join-Path $Root "tools/cpp_vm_trace_evidence"
$runningOnWindows = $env:OS -eq "Windows_NT"
$executableDirectory = if ($runningOnWindows) {
    Join-Path $buildDirectory $Configuration
} else {
    $buildDirectory
}
$appName = if ($runningOnWindows) { "lua_app.exe" } else { "lua_app" }
$bytecodeName = if ($runningOnWindows) { "lua_bytecode.exe" } else { "lua_bytecode" }
$evidenceName = if ($runningOnWindows) {
    "lua_bytecode_evidence.exe"
} else {
    "lua_bytecode_evidence"
}
$traceEvidenceName = if ($runningOnWindows) {
    "lua_vm_trace_evidence.exe"
} else {
    "lua_vm_trace_evidence"
}
$appPath = Join-Path $executableDirectory $appName
$bytecodePath = Join-Path $executableDirectory $bytecodeName
$evidencePath = Join-Path $executableDirectory $evidenceName
$traceEvidencePath = Join-Path $executableDirectory $traceEvidenceName

$document = [ordered]@{
    schemaVersion = 1
    channel = "cpp-oracle-build"
    passed = $false
    repository = $expectedRepository
    expectedCommit = $expectedCommit
    actualCommit = ""
    checkout = $cppPath
    configuration = $Configuration
    executables = [ordered]@{
        luaApp = [ordered]@{ path = $appPath; sha256 = "" }
        luaBytecode = [ordered]@{ path = $bytecodePath; sha256 = "" }
        luaBytecodeEvidence = [ordered]@{ path = $evidencePath; sha256 = "" }
        luaVmTraceEvidence = [ordered]@{ path = $traceEvidencePath; sha256 = "" }
    }
    evidenceAdapter = [ordered]@{
        sourceDirectory = $evidenceSourceDirectory
        cmakeSha256 = ""
        sourceSha256 = ""
    }
    traceEvidenceAdapter = [ordered]@{
        sourceDirectory = $traceEvidenceSourceDirectory
        cmakeSha256 = ""
        sourceSha256 = ""
    }
    failure = $null
}

try {
    if (-not (Test-Path -LiteralPath (Join-Path $cppPath ".git"))) {
        throw "Missing C++ oracle checkout: $cppPath"
    }
    $actualCommit = (& git -C $cppPath rev-parse HEAD 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "Could not read C++ oracle commit"
    }
    $document.actualCommit = $actualCommit
    if ($actualCommit -ne $expectedCommit) {
        throw "C++ oracle commit mismatch: expected $expectedCommit, got $actualCommit"
    }
    $actualRepository = (
        & git -C $cppPath remote get-url origin 2>$null | Out-String
    ).Trim()
    if ($LASTEXITCODE -ne 0 -or
        (Normalize-GitUrl $actualRepository) -ne
        (Normalize-GitUrl $expectedRepository)) {
        throw "C++ oracle repository mismatch: $actualRepository"
    }

    if (-not (Test-Path -LiteralPath $outputRoot)) {
        New-Item -ItemType Directory -Path $outputRoot -Force | Out-Null
    }
    $configureArguments = @(
        "-S", $cppPath,
        "-B", $buildDirectory,
        "-DLUA_CPP_BUILD_TESTS=OFF",
        "-DBUILD_TESTING=OFF",
        "-DLUA_CPP_BUILD_BENCHMARKS=OFF",
        "-DLUA_CPP_BUILD_SHARED=OFF"
    )
    if ($runningOnWindows) {
        $configureArguments += @("-A", "x64")
    } else {
        $configureArguments += "-DCMAKE_BUILD_TYPE=$Configuration"
    }
    & cmake @configureArguments
    if ($LASTEXITCODE -ne 0) {
        throw "CMake configure failed for the C++ oracle"
    }

    & cmake --build $buildDirectory --config $Configuration `
        --target lua_app lua_bytecode
    if ($LASTEXITCODE -ne 0) {
        throw "CMake build failed for the C++ oracle"
    }
    foreach ($executable in @($appPath, $bytecodePath)) {
        if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
            throw "C++ oracle executable was not produced: $executable"
        }
    }

    & cmake --install $buildDirectory --config $Configuration `
        --prefix $installDirectory
    if ($LASTEXITCODE -ne 0) {
        throw "CMake install failed for the pinned C++ oracle library"
    }

    $evidenceConfigureArguments = @(
        "-S", $evidenceSourceDirectory,
        "-B", $evidenceBuildDirectory,
        "-DLUA_CPP_SOURCE_ROOT=$cppPath",
        "-DLUA_CPP_PACKAGE_ROOT=$installDirectory",
        "-DLUA_CPP_EVIDENCE_OUTPUT_DIRECTORY=$executableDirectory"
    )
    if ($runningOnWindows) {
        $evidenceConfigureArguments += @("-A", "x64")
    } else {
        $evidenceConfigureArguments += "-DCMAKE_BUILD_TYPE=$Configuration"
    }
    & cmake @evidenceConfigureArguments
    if ($LASTEXITCODE -ne 0) {
        throw "CMake configure failed for the C++ bytecode evidence adapter"
    }

    & cmake --build $evidenceBuildDirectory --config $Configuration `
        --target lua_bytecode_evidence
    if ($LASTEXITCODE -ne 0) {
        throw "CMake build failed for the C++ bytecode evidence adapter"
    }
    if (-not (Test-Path -LiteralPath $evidencePath -PathType Leaf)) {
        throw "C++ bytecode evidence executable was not produced: $evidencePath"
    }

    $traceEvidenceConfigureArguments = @(
        "-S", $traceEvidenceSourceDirectory,
        "-B", $traceEvidenceBuildDirectory,
        "-DLUA_CPP_SOURCE_ROOT=$cppPath",
        "-DLUA_CPP_PACKAGE_ROOT=$installDirectory",
        "-DLUA_CPP_EVIDENCE_OUTPUT_DIRECTORY=$executableDirectory"
    )
    if ($runningOnWindows) {
        $traceEvidenceConfigureArguments += @("-A", "x64")
    } else {
        $traceEvidenceConfigureArguments += "-DCMAKE_BUILD_TYPE=$Configuration"
    }
    & cmake @traceEvidenceConfigureArguments
    if ($LASTEXITCODE -ne 0) {
        throw "CMake configure failed for the C++ VM trace evidence adapter"
    }

    & cmake --build $traceEvidenceBuildDirectory --config $Configuration `
        --target lua_vm_trace_evidence
    if ($LASTEXITCODE -ne 0) {
        throw "CMake build failed for the C++ VM trace evidence adapter"
    }
    if (-not (Test-Path -LiteralPath $traceEvidencePath -PathType Leaf)) {
        throw "C++ VM trace evidence executable was not produced: $traceEvidencePath"
    }

    $document.executables.luaApp.sha256 = (
        Get-FileHash -LiteralPath $appPath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $document.executables.luaBytecode.sha256 = (
        Get-FileHash -LiteralPath $bytecodePath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $document.executables.luaBytecodeEvidence.sha256 = (
        Get-FileHash -LiteralPath $evidencePath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $document.executables.luaVmTraceEvidence.sha256 = (
        Get-FileHash -LiteralPath $traceEvidencePath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $document.evidenceAdapter.cmakeSha256 = (
        Get-FileHash -LiteralPath (
            Join-Path $evidenceSourceDirectory "CMakeLists.txt"
        ) -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $document.evidenceAdapter.sourceSha256 = (
        Get-FileHash -LiteralPath (
            Join-Path $evidenceSourceDirectory "bytecode_evidence_main.cpp"
        ) -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $document.traceEvidenceAdapter.cmakeSha256 = (
        Get-FileHash -LiteralPath (
            Join-Path $traceEvidenceSourceDirectory "CMakeLists.txt"
        ) -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $document.traceEvidenceAdapter.sourceSha256 = (
        Get-FileHash -LiteralPath (
            Join-Path $traceEvidenceSourceDirectory "vm_trace_evidence_main.cpp"
        ) -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $document.passed = $true
} catch {
    $document.failure = $_.Exception.Message
}

$writtenResult = Write-Result $document
Write-Host "[INFO] C++ oracle build report: $writtenResult"
if (-not $document.passed) {
    Write-Host "[FAIL] $($document.failure)"
    exit 1
}

Write-Host (
    "[OK] Pinned C++ oracle tools built: " +
    "$appPath, $bytecodePath, $evidencePath, $traceEvidencePath"
)
