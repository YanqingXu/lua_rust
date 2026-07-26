<#
Internal deterministic process fixture for compare_vm_trace.ps1.
It is not a Lua implementation and must never be used for a parity result.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$TracePath,

    [Parameter(Mandatory = $true)]
    [string]$InputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$traceDirectory = Split-Path -Parent $TracePath
if ($traceDirectory -and -not (Test-Path -LiteralPath $traceDirectory -PathType Container)) {
    New-Item -ItemType Directory -Path $traceDirectory -Force | Out-Null
}

$sourceName = [System.IO.Path]::GetFileName($InputPath)
$events = @(
    [ordered]@{
        seq = 0; kind = "call"; funcName = $sourceName; source = $sourceName
        line = 1; callDepth = 1; stackTop = 0
    },
    [ordered]@{
        seq = 1; kind = "upvalue-open"; funcName = $sourceName; source = $sourceName
        line = 1; callDepth = 1; slot = 0; name = "fixture_upvalue"; stackTop = 1
    },
    [ordered]@{
        seq = 2; kind = "instruction"; funcName = $sourceName; source = $sourceName
        pc = 0; op = "LOADK"; a = 0; b = 0; c = 0; bx = 0; sbx = -131071
        line = 1; callDepth = 1; stackTop = 1
        changedRegisters = @(
            [ordered]@{
                slot = 0; name = "fixture"; old = $null; new = 1
                oldType = "nil"; newType = "number"
            }
        )
    },
    [ordered]@{
        seq = 3; kind = "yield"; funcName = $sourceName; source = $sourceName
        line = 1; callDepth = 1; stackTop = 1
    },
    [ordered]@{
        seq = 4; kind = "resume"; funcName = $sourceName; source = $sourceName
        line = 1; callDepth = 1; stackTop = 1
    },
    [ordered]@{
        seq = 5; kind = "upvalue-close"; funcName = $sourceName; source = $sourceName
        line = 1; callDepth = 1; slot = 0; name = "fixture_upvalue"; stackTop = 1
    },
    [ordered]@{
        seq = 6; kind = "error"; funcName = $sourceName; source = $sourceName
        line = 1; callDepth = 1; stackTop = 1
        errorValue = "fixture-error"; errorCategory = "fixture"
    },
    [ordered]@{
        seq = 7; kind = "return"; funcName = $sourceName; source = $sourceName
        line = 1; callDepth = 1; stackTop = 0
    }
)

$encoding = New-Object System.Text.UTF8Encoding($false)
$lines = @($events | ForEach-Object { $_ | ConvertTo-Json -Depth 16 -Compress })
[System.IO.File]::WriteAllText($TracePath, (($lines -join "`n") + "`n"), $encoding)

Write-Output "fixture-stdout"
[Console]::Error.WriteLine("fixture-stderr")
exit 0
