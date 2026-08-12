[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$CppAppExe,

    [Parameter(Mandatory = $true)]
    [string]$RustAppExe,

    [string]$OutputDir = "target/parity/cli-contract",

    [ValidateRange(1, 3600)]
    [int]$TimeoutSeconds = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDirectory ".."))
. (Join-Path $scriptDirectory "parity_runner_common.ps1")

if ($PSVersionTable.PSVersion.Major -lt 6) {
    $pwsh = Get-Command pwsh -ErrorAction SilentlyContinue
    if ($null -ne $pwsh) {
        $arguments = @(
            "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $MyInvocation.MyCommand.Path,
            "-CppAppExe", $CppAppExe, "-RustAppExe", $RustAppExe,
            "-OutputDir", $OutputDir, "-TimeoutSeconds", $TimeoutSeconds
        )
        & $pwsh.Source @arguments
        exit $LASTEXITCODE
    }
    throw "test_cli_contract requires PowerShell 6+ for BOM-free redirected stdin"
}

$CppAppExe = Resolve-ParityPath -Path $CppAppExe -BasePath $projectRoot
$RustAppExe = Resolve-ParityPath -Path $RustAppExe -BasePath $projectRoot
$OutputDir = Resolve-ParityPath -Path $OutputDir -BasePath $projectRoot
$resultPath = Join-Path $OutputDir "report.json"
New-ParityDirectory -Path $OutputDir

function ConvertTo-CliText {
    param(
        [AllowEmptyString()]
        [string]$Text,
        [Parameter(Mandatory = $true)]
        [string]$Executable
    )

    $normalized = $Text.Replace("`r`n", "`n")
    $normalized = $normalized.Replace($Executable, "<exe>")
    $basename = [System.IO.Path]::GetFileName($Executable)
    if (-not [string]::IsNullOrEmpty($basename)) {
        $normalized = $normalized.Replace($basename, "<exe>")
    }
    return $normalized
}

$utf8 = [System.Text.UTF8Encoding]::new($false)
$cases = @(
    [ordered]@{
        id = "version"
        arguments = @("-v")
        stdin = $null
        exitCode = 0
        stdout = "Lua C++ Interpreter v0.1.0`nBased on Lua 5.1.5 specification`nCopyright (C) 2025 Lua C++ Project`n"
        stderr = ""
    },
    [ordered]@{
        id = "help"
        arguments = @("-h")
        stdin = $null
        exitCode = 1
        stdout = "Usage: <exe> [options] [script [args]]`nAvailable options are:`n  -v       show version information`n  -e stat  execute string 'stat'`n  -l name  require library 'name'`n  -i       enter interactive mode`n  --trace <file>  write execution trace to JSONL file`n  --trace-diff <file>  write execution trace with changedRegisters`n  --       stop handling options`n  -        execute stdin`n"
        stderr = ""
    },
    [ordered]@{ id = "bad-option"; arguments = @("-f"); stdin = $null; exitCode = 1; stdout = ""; stderr = "<exe>: unrecognized option`n" },
    [ordered]@{ id = "missing-e"; arguments = @("-e"); stdin = $null; exitCode = 1; stdout = ""; stderr = "<exe>: '-e' needs argument`n" },
    [ordered]@{ id = "missing-l"; arguments = @("-l"); stdin = $null; exitCode = 1; stdout = ""; stderr = "<exe>: '-l' needs argument`n" },
    [ordered]@{ id = "missing-trace"; arguments = @("--trace"); stdin = $null; exitCode = 1; stdout = ""; stderr = "<exe>: trace option needs file argument`n" },
    [ordered]@{ id = "missing-trace-diff"; arguments = @("--trace-diff"); stdin = $null; exitCode = 1; stdout = ""; stderr = "<exe>: trace option needs file argument`n" },
    [ordered]@{
        id = "startup-action-order"
        arguments = @("-e", "a=1", "-ea=a+1", "-e", "io.write(a)")
        stdin = $null
        exitCode = 0
        stdout = "2"
        stderr = ""
    },
    [ordered]@{
        id = "require-actions"
        arguments = @("-lstring", "-l", "io", "-e", "io.write(type(string),'|',type(io))")
        stdin = $null
        exitCode = 0
        stdout = "table|table"
        stderr = ""
    },
    [ordered]@{
        id = "stdin-hash-bang-source"
        arguments = @("-")
        stdin = $utf8.GetBytes("#!/usr/bin/env lua`r`nio.write(debug.getinfo(1).source,'|',debug.getinfo(1).currentline)")
        exitCode = 0
        stdout = "=stdin|2"
        stderr = ""
    },
    [ordered]@{
        id = "stdin-syntax-line"
        arguments = @("-")
        stdin = $utf8.GetBytes("#!/usr/bin/env lua`nreturn return")
        exitCode = 1
        stdout = ""
        stderr = "<exe>: stdin:2: unexpected symbol near 'return'`n"
    },
    [ordered]@{
        id = "stdin-script-args"
        arguments = @("-", "alpha", "beta")
        stdin = $utf8.GetBytes("io.write(arg[0],'|',arg[1],'|',arg[2],'|',...)")
        exitCode = 0
        stdout = "-|alpha|beta|alphabeta"
        stderr = ""
    },
    [ordered]@{
        id = "double-dash-script-args"
        arguments = @("-e ", "--", "tests/cli/argument_contract.lua", "alpha", "beta")
        stdin = $null
        exitCode = 0
        stdout = "-e |--|tests/cli/argument_contract.lua|alpha|beta|alpha|beta"
        stderr = ""
    },
    [ordered]@{
        id = "file-hash-bang-source"
        arguments = @("tests/cli/hash_bang_source.lua")
        stdin = $null
        exitCode = 0
        stdout = "@tests/cli/hash_bang_source.lua|2"
        stderr = ""
    },
    [ordered]@{
        id = "file-syntax-source"
        arguments = @("tests/lua/basic/syntax_error.lua")
        stdin = $null
        exitCode = 1
        stdout = ""
        stderr = "<exe>: tests/lua/basic/syntax_error.lua:7: unexpected symbol near '<eof>'`n"
    },
    [ordered]@{
        id = "missing-file"
        arguments = @("__m216_missing_script__.lua")
        stdin = $null
        exitCode = 1
        stdout = ""
        stderr = "<exe>: cannot open file: __m216_missing_script__.lua`n"
    },
    [ordered]@{
        id = "interactive-prompts"
        arguments = @("-e", "_PROMPT='';_PROMPT2=''", "-i")
        stdin = $utf8.GetBytes("=6*7`n")
        exitCode = 0
        stdout = "42`n`n"
        stderr = ""
    },
    [ordered]@{
        id = "interactive-error-recovery"
        arguments = @("-e", "_PROMPT='';_PROMPT2=''", "-i")
        stdin = $utf8.GetBytes("return return`n=2`nerror('boom')`n=3`n")
        exitCode = 0
        stdout = "2`n3`n`n"
        stderr = "stdin:1: unexpected symbol near 'return'`n(repl):1: boom`n"
    }
)

$issues = New-Object System.Collections.ArrayList
$caseReports = @()
foreach ($case in $cases) {
    $caseDirectory = Join-Path $OutputDir $case.id
    New-ParityDirectory -Path $caseDirectory
    $cppExecution = Invoke-ParityProcess -Executable $CppAppExe `
        -Arguments $case.arguments -StandardInputBytes $case.stdin `
        -WorkingDirectory $projectRoot -TimeoutSeconds $TimeoutSeconds
    $rustExecution = Invoke-ParityProcess -Executable $RustAppExe `
        -Arguments $case.arguments -StandardInputBytes $case.stdin `
        -WorkingDirectory $projectRoot -TimeoutSeconds $TimeoutSeconds
    $cppSaved = Save-ParityExecution -Execution $cppExecution -CaseDirectory $caseDirectory -Side "cpp"
    $rustSaved = Save-ParityExecution -Execution $rustExecution -CaseDirectory $caseDirectory -Side "rust"
    $cppStdout = ConvertTo-CliText -Text $cppSaved.stdout -Executable $CppAppExe
    $cppStderr = ConvertTo-CliText -Text $cppSaved.stderr -Executable $CppAppExe
    $rustStdout = ConvertTo-CliText -Text $rustSaved.stdout -Executable $RustAppExe
    $rustStderr = ConvertTo-CliText -Text $rustSaved.stderr -Executable $RustAppExe
    $caseIssues = New-Object System.Collections.ArrayList
    foreach ($side in @(
        [ordered]@{ name = "cpp"; execution = $cppSaved; stdout = $cppStdout; stderr = $cppStderr },
        [ordered]@{ name = "rust"; execution = $rustSaved; stdout = $rustStdout; stderr = $rustStderr }
    )) {
        if ($side.execution.timedOut) {
            [void]$caseIssues.Add("$($side.name) timed out")
        }
        if ($null -ne $side.execution.startError) {
            [void]$caseIssues.Add("$($side.name) start error: $($side.execution.startError)")
        }
        if ($side.execution.exitCode -ne $case.exitCode) {
            [void]$caseIssues.Add("$($side.name) exit $($side.execution.exitCode), expected $($case.exitCode)")
        }
        if ($side.stdout -cne $case.stdout) {
            [void]$caseIssues.Add("$($side.name) stdout did not match the fixed contract")
        }
        if ($side.stderr -cne $case.stderr) {
            [void]$caseIssues.Add("$($side.name) stderr did not match the fixed contract")
        }
    }
    if ($cppSaved.exitCode -ne $rustSaved.exitCode -or
        $cppStdout -cne $rustStdout -or $cppStderr -cne $rustStderr) {
        [void]$caseIssues.Add("cross-language process evidence differs")
    }
    foreach ($caseIssue in $caseIssues) {
        [void]$issues.Add("$($case.id): $caseIssue")
    }
    $caseReports += [ordered]@{
        id = $case.id
        status = if ($caseIssues.Count -eq 0) { "passed" } else { "failed" }
        arguments = @($case.arguments)
        expected = [ordered]@{ exitCode = $case.exitCode; stdout = $case.stdout; stderr = $case.stderr }
        normalized = [ordered]@{
            cpp = [ordered]@{ stdout = $cppStdout; stderr = $cppStderr }
            rust = [ordered]@{ stdout = $rustStdout; stderr = $rustStderr }
        }
        executions = [ordered]@{ cpp = $cppSaved; rust = $rustSaved }
        issues = @($caseIssues)
    }
}

$passed = @($caseReports | Where-Object { $_.status -eq "passed" }).Count
$report = [ordered]@{
    schemaVersion = 1
    runner = "test_cli_contract"
    purpose = "cross-language-cli-contract"
    status = if ($issues.Count -eq 0) { "passed" } else { "failed" }
    summary = [ordered]@{
        selected = $cases.Count
        passed = $passed
        failed = $cases.Count - $passed
        infrastructureFailures = @($caseReports | Where-Object {
            $_.executions.cpp.timedOut -or $_.executions.rust.timedOut -or
            $null -ne $_.executions.cpp.startError -or $null -ne $_.executions.rust.startError
        }).Count
    }
    covered = @("arguments", "stdin", "-e", "-l", "-i", "--", "script-args", "hash-bang", "file-errors", "source-name", "stdout-stderr-exit")
    deferred = @([ordered]@{
        id = "binary-chunk-execution"
        milestone = "M3"
        reason = "Rust binary chunk serializer/deserializer is not implemented"
    })
    cases = @($caseReports)
    issues = @($issues)
}
Write-ParityJson -Path $resultPath -Value $report

if ($issues.Count -ne 0) {
    foreach ($issue in $issues) {
        [Console]::Error.WriteLine("[FAIL] $issue")
    }
    [Console]::Error.WriteLine("CLI contract test failed: $resultPath")
    exit 1
}

Write-Output "[PASS] CLI contract: $passed/$($cases.Count) cases"
Write-Output "  report=$resultPath"
exit 0
