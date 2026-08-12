[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$CppAppExe,

    [Parameter(Mandatory = $true)]
    [string]$RustAppExe,

    [string]$OutputDir = "target/parity/repl-contract",

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
    throw "test_repl_contract requires PowerShell 6+ for BOM-free redirected stdin"
}

$CppAppExe = Resolve-ParityPath -Path $CppAppExe -BasePath $projectRoot
$RustAppExe = Resolve-ParityPath -Path $RustAppExe -BasePath $projectRoot
$OutputDir = Resolve-ParityPath -Path $OutputDir -BasePath $projectRoot
$resultPath = Join-Path $OutputDir "report.json"
New-ParityDirectory -Path $OutputDir

$banner = "Lua 5.1.5  Copyright (C) 1994-2012 Lua.org, PUC-Rio`n"
$help = "REPL commands:`n  .help                  show this help`n  .bytecode <expr|chunk> compile input and print bytecode`n  .ast <expr|chunk>      parse input and print AST`n  .gc [stats|collect|strategy|help] inspect or run the active GC`n  =expr                  evaluate expression and print results`n  exit, quit             leave the REPL`n"
$bytecode = "Proto`n  source: =(repl bytecode)`n  linedefined: 0`n  lastlinedefined: 0`n  numparams: 0`n  is_vararg: true (flags=2)`n  maxStackSize: 2`n  upvalues (0): (none)`ninstructions (3)`n0000 | line 1 | LOADK | A=0 Bx=0 ; K[0] = number 3`n0001 | line 1 | RETURN | A=0 B=2 C=0`n0002 | line 1 | RETURN | A=0 B=1 C=0`nconstants (1)`n  K[0] = number 3`n"
$ast = "AST`n  mode: expression`nChunk`n  statements (1)`n    [0]`n      ReturnStmt @ 1:1`n        values (1)`n          [0]`n            BinaryExpr op=Add @ 1:9`n              left:`n                NumberExpr value=1 @ 1:8`n              right:`n                NumberExpr value=2 @ 1:10`n"
$strategy = "GC`n  command: strategy`n  active: mark-sweep`n  active summary: stop-the-world tri-color mark and sweep`n  available: mark-sweep, incremental`n  planned: incremental write barriers and scheduling`n  boundary: RuntimeServices.gc owns the active collector`n  switch: collectgarbage(`"strategy`", `"mark-sweep`"|`"incremental`")`n"

$cases = @(
    [ordered]@{
        id = "help-history-goodbye"
        stdin = ".help`n.unknown`n=1+2`nquit`n"
        stdout = $banner + "> " + $help + "> > 3`n> Goodbye!`n"
        stderr = "unknown REPL command: .unknown`n"
    },
    [ordered]@{
        id = "multiline-expression-reset"
        stdin = "if true then`nio.write('block')`nend`n=6*7`nquit`n"
        stdout = $banner + "> >> >> block> 42`n> Goodbye!`n"
        stderr = ""
    },
    [ordered]@{
        id = "syntax-runtime-recovery"
        stdin = "return return`n=2`nerror('boom')`n=3`nquit`n"
        stdout = $banner + "> > 2`n> > 3`n> Goodbye!`n"
        stderr = "stdin:1: unexpected symbol near 'return'`n(repl):1: boom`n"
    },
    [ordered]@{
        id = "bare-name-reset"
        stdin = "pending`n=9`npending=4`n=pending`nquit`n"
        stdout = $banner + "> > 9`n> > 4`n> Goodbye!`n"
        stderr = "stdin:1: unexpected symbol near 'pending'`n"
    },
    [ordered]@{
        id = "incomplete-eof"
        stdin = "if true then`n"
        stdout = $banner + "> >> `nGoodbye!`n"
        stderr = ""
    },
    [ordered]@{
        id = "bytecode-ast"
        stdin = ".bytecode 1+2`n.ast 1+2`nquit`n"
        stdout = $banner + "> " + $bytecode + "> " + $ast + "> Goodbye!`n"
        stderr = ""
    },
    [ordered]@{
        id = "meta-errors-gc"
        stdin = ".bytecode return return`n.ast return return`n.gc help`n.gc strategy`n.gc nope`nquit`n"
        stdout = $banner + "> > > usage: .gc [stats|collect|strategy|help]`n> " + $strategy + "> > Goodbye!`n"
        stderr = "stdin:1: unexpected symbol near 'return'`nstdin:1: unexpected symbol near 'return'`n.gc: unknown option 'nope'`nusage: .gc [stats|collect|strategy|help]`n"
    },
    [ordered]@{
        id = "completion"
        stdin = ".g`t help`npri`t('ok')`n=string.su`t('abcd',2)`n.gc st`t`nquit`n"
        history = ".gc help`nprint('ok')`n=string.sub('abcd',2)`n.gc st`nquit`n"
        stdout = $banner + "> usage: .gc [stats|collect|strategy|help]`n> ok`n> bcd`n> `nstats  status  strategy`n> Goodbye!`n"
        stderr = ".gc: unknown option 'st'`nusage: .gc [stats|collect|strategy|help]`n"
    }
)

$utf8 = [System.Text.UTF8Encoding]::new($false)
$issues = New-Object System.Collections.ArrayList
$caseReports = @()
foreach ($case in $cases) {
    $caseDirectory = Join-Path $OutputDir $case.id
    New-ParityDirectory -Path $caseDirectory
    $sideReports = [ordered]@{}
    foreach ($side in @(
        [ordered]@{ name = "cpp"; executable = $CppAppExe },
        [ordered]@{ name = "rust"; executable = $RustAppExe }
    )) {
        $sideDirectory = Join-Path $caseDirectory $side.name
        New-ParityDirectory -Path $sideDirectory
        $execution = Invoke-ParityProcess -Executable $side.executable `
            -Arguments @() -StandardInputBytes $utf8.GetBytes($case.stdin) `
            -WorkingDirectory $sideDirectory -TimeoutSeconds $TimeoutSeconds
        $saved = Save-ParityExecution -Execution $execution -CaseDirectory $caseDirectory -Side $side.name
        $stdout = $saved.stdout.Replace("`r`n", "`n")
        $stderr = $saved.stderr.Replace("`r`n", "`n")
        $historyPath = Join-Path $sideDirectory ".lua_history"
        $history = if (Test-Path -LiteralPath $historyPath) {
            [System.IO.File]::ReadAllText($historyPath).Replace("`r`n", "`n")
        } else {
            $null
        }
        $sideReports[$side.name] = [ordered]@{
            execution = $saved
            stdout = $stdout
            stderr = $stderr
            history = $history
        }
    }

    $caseIssues = New-Object System.Collections.ArrayList
    foreach ($sideName in @("cpp", "rust")) {
        $side = $sideReports[$sideName]
        if ($side.execution.timedOut) { [void]$caseIssues.Add("$sideName timed out") }
        if ($null -ne $side.execution.startError) { [void]$caseIssues.Add("$sideName start error: $($side.execution.startError)") }
        if ($side.execution.exitCode -ne 0) { [void]$caseIssues.Add("$sideName exit $($side.execution.exitCode), expected 0") }
        if ($side.stdout -cne $case.stdout) { [void]$caseIssues.Add("$sideName stdout did not match fixed REPL contract") }
        if ($side.stderr -cne $case.stderr) { [void]$caseIssues.Add("$sideName stderr did not match fixed REPL contract") }
        $expectedHistory = if ($case.Contains("history")) { $case.history } else { $case.stdin }
        if ($side.history -cne $expectedHistory) { [void]$caseIssues.Add("$sideName history did not preserve the fixed completed-line contract") }
    }
    if ($sideReports.cpp.stdout -cne $sideReports.rust.stdout -or
        $sideReports.cpp.stderr -cne $sideReports.rust.stderr -or
        $sideReports.cpp.history -cne $sideReports.rust.history) {
        [void]$caseIssues.Add("cross-language REPL evidence differs")
    }
    foreach ($issue in $caseIssues) { [void]$issues.Add("$($case.id): $issue") }
    $caseReports += [ordered]@{
        id = $case.id
        status = if ($caseIssues.Count -eq 0) { "passed" } else { "failed" }
        executions = $sideReports
        issues = @($caseIssues)
    }
}

$passed = @($caseReports | Where-Object { $_.status -eq "passed" }).Count
$report = [ordered]@{
    schemaVersion = 1
    runner = "test_repl_contract"
    purpose = "cross-language-repl-contract"
    status = if ($issues.Count -eq 0) { "passed" } else { "failed" }
    summary = [ordered]@{
        selected = $cases.Count
        passed = $passed
        failed = $cases.Count - $passed
    }
    covered = @("multiline", "incomplete-eof", "expression", "syntax-runtime-recovery", "state-stack-reset", "history", "meta-command", "submitted-completion", "prompt", "stdout-stderr-exit")
    terminalOnly = @("interactive-console-tab", "backspace", "ctrl-c", "ctrl-d-z")
    cases = @($caseReports)
    issues = @($issues)
}
Write-ParityJson -Path $resultPath -Value $report

if ($issues.Count -ne 0) {
    foreach ($issue in $issues) { [Console]::Error.WriteLine("[FAIL] $issue") }
    [Console]::Error.WriteLine("REPL contract test failed: $resultPath")
    exit 1
}

Write-Output "[PASS] REPL contract: $passed/$($cases.Count) cases"
Write-Output "  report=$resultPath"
exit 0
