<#
.SYNOPSIS
    Lua Rust — Environment Check
.DESCRIPTION
    Checks that all required Rust tools and workspace crates are available.
.PARAMETER InstallTools
    Attempt to install missing tools automatically.
.PARAMETER Verbose
    Show detailed version output for each tool.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_env_check.ps1
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_env_check.ps1 -InstallTools -Verbose
#>

param(
    [switch]$InstallTools,
    [switch]$Verbose
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Resolve-Path (Join-Path $ScriptDir "..")

$RequiredTools = @(
    @{
        Name      = "rustup"
        Command   = "rustup"
        Arguments = @("--version")
        Install   = { Write-Host "    Install via: winget install Rustlang.Rustup" -ForegroundColor Yellow }
    },
    @{
        Name      = "cargo"
        Command   = "cargo"
        Arguments = @("--version")
        Install   = { Write-Host "    Included with rustup" -ForegroundColor Yellow }
    },
    @{
        Name      = "rustc"
        Command   = "rustc"
        Arguments = @("--version")
        Install   = { Write-Host "    Included with rustup" -ForegroundColor Yellow }
    },
    @{
        Name      = "clippy"
        Command   = "cargo"
        Arguments = @("clippy", "--version")
        Install   = { rustup component add clippy }
    },
    @{
        Name      = "rustfmt"
        Command   = "rustfmt"
        Arguments = @("--version")
        Install   = { rustup component add rustfmt }
    },
    @{
        Name      = "cargo-nextest"
        Command   = "cargo"
        Arguments = @("nextest", "--version")
        Install   = { cargo install cargo-nextest }
    },
    @{
        Name      = "cargo-audit"
        Command   = "cargo"
        Arguments = @("audit", "--version")
        Install   = { cargo install cargo-audit }
    }
)

Write-Host "=== Lua Rust Environment Check ===" -ForegroundColor Cyan
Write-Host "  Project root: $ProjectRoot`n"

$AllOk = $true

foreach ($tool in $RequiredTools) {
    $name = $tool.Name
    try {
        $command = Get-Command $tool["Command"] -CommandType Application `
            -ErrorAction Stop | Select-Object -First 1
        $arguments = @($tool["Arguments"])
        $oldPreference = $ErrorActionPreference
        try {
            # Windows PowerShell 5.1 promotes native stderr to an error record.
            # Some successful version probes (notably rustup) write notices to
            # stderr, so determine success from the native exit code instead.
            $ErrorActionPreference = "Continue"
            $output = @(& $command.Source @arguments 2>&1)
            $exitCode = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = $oldPreference
        }
        if ($exitCode -ne 0) {
            throw "$name exited with status $exitCode"
        }
        Write-Host "  [OK] $name" -ForegroundColor Green
        if ($Verbose) {
            Write-Host "       $($output -join "`n       ")" -ForegroundColor Gray
        }
    }
    catch {
        Write-Host "  [MISSING] $name" -ForegroundColor Red
        $AllOk = $false
        if ($InstallTools) {
            Write-Host "    Installing $name..." -ForegroundColor Yellow
            try {
                $install = $tool["Install"]
                & $install
                Write-Host "    [OK] $name installed" -ForegroundColor Green
            }
            catch {
                Write-Host "    [FAIL] Could not install $name : $_" -ForegroundColor Red
            }
        }
    }
}

# Check workspace structure
Write-Host "`n=== Workspace Structure ===" -ForegroundColor Cyan
$crates = @("lua_core", "lua_compiler", "lua_vm", "lua_stdlib", "lua_app", "lua_bytecode")
foreach ($crate in $crates) {
    $path = Join-Path $ProjectRoot "crates/$crate/Cargo.toml"
    if (Test-Path $path) {
        Write-Host "  [OK] crates/$crate/" -ForegroundColor Green
    }
    else {
        Write-Host "  [MISSING] crates/$crate/" -ForegroundColor Red
        $AllOk = $false
    }
}

# Summary
Write-Host "`n========================================" -ForegroundColor Cyan
if ($AllOk) {
    Write-Host "  Status: ALL READY" -ForegroundColor Green
    Write-Host "  Next: cargo build --workspace" -ForegroundColor Gray
    exit 0
}
else {
    Write-Host "  Status: SOME TOOLS/CRATES MISSING" -ForegroundColor Red
    Write-Host "  Re-run with -InstallTools to auto-install missing Rust components" -ForegroundColor Yellow
    exit 1
}
