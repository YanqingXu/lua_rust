param(
    [string]$Root = "",
    [string]$OutputDirectory = "target/oracles/lua-5.1.5",
    [string]$ResultPath = "target/compatibility/lua51-oracle-build.json"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$sourceUrl = "https://www.lua.org/ftp/lua-5.1.5.tar.gz"
$sourceSha256 = "2640fc56a795f29d28ef15e13c34a47e223960b0240e8cb0a82d9b0738695333"

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

function Get-ExecutableVersion {
    param([Parameter(Mandatory = $true)][string]$Executable)
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.Arguments = "-v"
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw "Could not start built Lua 5.1.5 executable"
        }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        return [ordered]@{
            exitCode = $process.ExitCode
            text = (($stdout.Result + $stderr.Result).Trim())
        }
    } finally {
        $process.Dispose()
    }
}

$outputRoot = Resolve-RootedPath $OutputDirectory
$archivePath = Join-Path $outputRoot "lua-5.1.5.tar.gz"
$sourceParent = Join-Path $outputRoot "source"
$sourceRoot = Join-Path $sourceParent "lua-5.1.5"
$sourceDirectory = Join-Path $sourceRoot "src"
$projectDirectory = Join-Path $outputRoot "cmake-project"
$buildDirectory = Join-Path $outputRoot "build"
$runningOnWindows = $env:OS -eq "Windows_NT"
$executablePath = if ($runningOnWindows) {
    Join-Path $buildDirectory "Release/lua51.exe"
} else {
    Join-Path $buildDirectory "lua51"
}

$document = [ordered]@{
    schemaVersion = 1
    channel = "lua51-oracle-build"
    passed = $false
    source = [ordered]@{
        url = $sourceUrl
        expectedSha256 = $sourceSha256
        archive = $archivePath
        actualSha256 = ""
    }
    executable = [ordered]@{
        path = $executablePath
        sha256 = ""
        version = ""
    }
    failure = $null
}

try {
    if (-not (Test-Path -LiteralPath $outputRoot)) {
        New-Item -ItemType Directory -Path $outputRoot -Force | Out-Null
    }
    if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
        Write-Host "[INFO] Downloading Lua 5.1.5 source from $sourceUrl"
        Invoke-WebRequest -Uri $sourceUrl -OutFile $archivePath -UseBasicParsing
    }

    $actualArchiveSha = (
        Get-FileHash -LiteralPath $archivePath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $document.source.actualSha256 = $actualArchiveSha
    if ($actualArchiveSha -ne $sourceSha256) {
        throw "Lua 5.1.5 source archive SHA-256 mismatch"
    }

    if (-not (Test-Path -LiteralPath $sourceDirectory -PathType Container)) {
        if (-not (Test-Path -LiteralPath $sourceParent)) {
            New-Item -ItemType Directory -Path $sourceParent -Force | Out-Null
        }
        & tar -xzf $archivePath -C $sourceParent
        if ($LASTEXITCODE -ne 0) {
            throw "Could not extract the Lua 5.1.5 source archive"
        }
    }

    if (-not (Test-Path -LiteralPath $projectDirectory)) {
        New-Item -ItemType Directory -Path $projectDirectory -Force | Out-Null
    }
    $cmakeSourcePath = $sourceDirectory.Replace("\", "/").Replace('"', '\"')
    $sourceNames = @(
        "lua.c",
        "lapi.c",
        "lcode.c",
        "ldebug.c",
        "ldo.c",
        "ldump.c",
        "lfunc.c",
        "lgc.c",
        "llex.c",
        "lmem.c",
        "lobject.c",
        "lopcodes.c",
        "lparser.c",
        "lstate.c",
        "lstring.c",
        "ltable.c",
        "ltm.c",
        "lundump.c",
        "lvm.c",
        "lzio.c",
        "lauxlib.c",
        "lbaselib.c",
        "ldblib.c",
        "liolib.c",
        "lmathlib.c",
        "loslib.c",
        "ltablib.c",
        "lstrlib.c",
        "loadlib.c",
        "linit.c"
    )
    $cmakeSources = ($sourceNames | ForEach-Object {
        "    `"`${LUA_SOURCE}/$_`""
    }) -join [Environment]::NewLine
    $cmakeDocument = @"
cmake_minimum_required(VERSION 3.20)
project(lua51_oracle VERSION 5.1.5 LANGUAGES C)
set(CMAKE_C_STANDARD 90)
set(CMAKE_C_STANDARD_REQUIRED OFF)
set(LUA_SOURCE "$cmakeSourcePath")
add_executable(lua51
$cmakeSources
)
target_include_directories(lua51 PRIVATE "`${LUA_SOURCE}")
if(WIN32)
    target_compile_definitions(lua51 PRIVATE LUA_USE_WINDOWS _CRT_SECURE_NO_WARNINGS)
endif()
"@
    $cmakeListsPath = Join-Path $projectDirectory "CMakeLists.txt"
    [System.IO.File]::WriteAllText(
        $cmakeListsPath,
        $cmakeDocument,
        [System.Text.UTF8Encoding]::new($false)
    )

    $configureArguments = @("-S", $projectDirectory, "-B", $buildDirectory)
    if ($runningOnWindows) {
        $configureArguments += @("-A", "x64")
    } else {
        $configureArguments += "-DCMAKE_BUILD_TYPE=Release"
    }
    & cmake @configureArguments
    if ($LASTEXITCODE -ne 0) {
        throw "CMake configure failed for the Lua 5.1.5 oracle"
    }

    & cmake --build $buildDirectory --config Release --target lua51
    if ($LASTEXITCODE -ne 0) {
        throw "CMake build failed for the Lua 5.1.5 oracle"
    }
    if (-not (Test-Path -LiteralPath $executablePath -PathType Leaf)) {
        throw "Lua 5.1.5 executable was not produced: $executablePath"
    }

    $version = Get-ExecutableVersion $executablePath
    if ($version.exitCode -ne 0 -or $version.text -notmatch '^Lua 5\.1\.5(?:\s|$)') {
        throw "Built reference executable is not Lua 5.1.5: $($version.text)"
    }

    $document.executable.sha256 = (
        Get-FileHash -LiteralPath $executablePath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $document.executable.version = $version.text
    $document.passed = $true
} catch {
    $document.failure = $_.Exception.Message
}

$writtenResult = Write-Result $document
Write-Host "[INFO] Lua 5.1.5 oracle build report: $writtenResult"
if (-not $document.passed) {
    Write-Host "[FAIL] $($document.failure)"
    exit 1
}

Write-Host "[OK] Lua 5.1.5 oracle built and provenance-checked: $executablePath"
