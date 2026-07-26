<#
.SYNOPSIS
    Compare lua_cpp and lua_rust compiler bytecode without treating missing evidence as success.
.DESCRIPTION
    Runs each selected Lua source through the C++ full-text dumper and the Rust JSON
    dumper. Every case receives an evidence directory containing the exact commands,
    stdout, stderr, exit status, timeout state, normalized bytecode, and structured
    differences. The default parity mode is fail-closed when a tool or required schema
    field is unavailable.

    Representative mode is intended for pull-request checks. Full mode recursively
    executes every Lua source and is intended for nightly checks.
.PARAMETER InputPath
    A .lua file or a directory containing Lua sources.
.PARAMETER CorpusMode
    Representative selects a deterministic bounded corpus; Full selects all sources.
.PARAMETER RepresentativeManifest
    Optional JSON array (or {"files":[]}) of paths relative to InputPath.
.PARAMETER InfrastructureSelfTest
    Runs the selected tool on both sides to prove capture, parsing, comparison, and
    artifact generation. It makes no cross-language parity claim and does not waive
    evidence requirements in normal mode.
.EXAMPLE
    pwsh -File tools/compare_bytecode.ps1 -InputPath tests/lua/bytecode
.EXAMPLE
    pwsh -File tools/compare_bytecode.ps1 -InputPath tests/lua -CorpusMode Full
.EXAMPLE
    pwsh -File tools/compare_bytecode.ps1 -InputPath tests/lua/bytecode/test_bytecode.lua `
        -InfrastructureSelfTest -SelfTestTool Cpp
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [Alias("InputDir")]
    [string]$InputPath,

    [ValidateSet("Representative", "Full")]
    [string]$CorpusMode = "Representative",

    [ValidateRange(1, 100000)]
    [int]$RepresentativeCount = 12,

    [string]$RepresentativeManifest = "",
    [string]$CppBytecodeExe = "",
    [string]$RustBytecodeExe = "",
    [string]$OutputDir = "",
    [string]$ResultPath = "",

    [ValidateRange(1, 3600)]
    [int]$TimeoutSeconds = 30,

    [ValidateSet("Cpp", "Rust")]
    [string]$SelfTestTool = "Cpp",

    [switch]$InfrastructureSelfTest,
    [switch]$JsonOutput
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDirectory ".."))
. (Join-Path $scriptDirectory "parity_runner_common.ps1")

function Get-OpcodeSpecification {
    $names = @(
        "MOVE", "LOADK", "LOADBOOL", "LOADNIL", "GETUPVAL", "GETGLOBAL",
        "GETTABLE", "SETGLOBAL", "SETUPVAL", "SETTABLE", "NEWTABLE", "SELF",
        "ADD", "SUB", "MUL", "DIV", "MOD", "POW", "UNM", "NOT", "LEN",
        "CONCAT", "JMP", "EQ", "LT", "LE", "TEST", "TESTSET", "CALL",
        "TAILCALL", "RETURN", "FORLOOP", "FORPREP", "TFORLOOP", "SETLIST",
        "CLOSE", "CLOSURE", "VARARG"
    )
    $abx = @("LOADK", "GETGLOBAL", "SETGLOBAL", "CLOSURE")
    $asbx = @("JMP", "FORLOOP", "FORPREP")
    $rk = @{
        GETTABLE = @("c")
        SETTABLE = @("b", "c")
        SELF     = @("c")
        ADD      = @("b", "c")
        SUB      = @("b", "c")
        MUL      = @("b", "c")
        DIV      = @("b", "c")
        MOD      = @("b", "c")
        POW      = @("b", "c")
        EQ       = @("b", "c")
        LT       = @("b", "c")
        LE       = @("b", "c")
    }

    $specification = @{}
    for ($index = 0; $index -lt $names.Count; $index++) {
        $name = $names[$index]
        $mode = if ($abx -contains $name) {
            "ABx"
        }
        elseif ($asbx -contains $name) {
            "AsBx"
        }
        else {
            "ABC"
        }
        $specification[$name] = [pscustomobject][ordered]@{
            code = $index
            mode = $mode
            rk   = if ($rk.ContainsKey($name)) { @($rk[$name]) } else { @() }
        }
    }
    return $specification
}

$opcodeSpecification = Get-OpcodeSpecification

function Get-RequiredPropertyValue {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Object,
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        throw "missing property '$Name'"
    }
    return $property.Value
}

function Convert-ByteArrayToHex {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [byte[]]$Bytes
    )

    $builder = [System.Text.StringBuilder]::new($Bytes.Count * 2)
    foreach ($byte in $Bytes) {
        [void]$builder.AppendFormat(
            [System.Globalization.CultureInfo]::InvariantCulture,
            "{0:x2}",
            $byte
        )
    }
    return $builder.ToString()
}

function Convert-Utf8TextToHex {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Text
    )

    return Convert-ByteArrayToHex -Bytes ([System.Text.Encoding]::UTF8.GetBytes($Text))
}

function Convert-CppEscapedStringToHex {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Text
    )

    # lua_cpp's bytecode printer escapes exactly these five byte sequences.
    # Decoding them before UTF-8 encoding gives both adapters one canonical,
    # byte-oriented comparison value for all text the oracle can represent.
    $decoded = [System.Text.StringBuilder]::new($Text.Length)
    for ($index = 0; $index -lt $Text.Length; $index++) {
        $character = $Text[$index]
        if ($character -ne '\') {
            [void]$decoded.Append($character)
            continue
        }
        if ($index + 1 -ge $Text.Length) {
            throw "C++ string evidence ends with an incomplete escape"
        }
        $index++
        switch ($Text[$index]) {
            '\' { [void]$decoded.Append('\') }
            '"' { [void]$decoded.Append('"') }
            'n' { [void]$decoded.Append([char]10) }
            'r' { [void]$decoded.Append([char]13) }
            't' { [void]$decoded.Append([char]9) }
            default {
                throw "C++ string evidence contains unsupported escape '\$($Text[$index])'"
            }
        }
    }
    return Convert-Utf8TextToHex -Text $decoded.ToString()
}

function Convert-RustByteEnvelopeToHex {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Envelope,
        [Parameter(Mandatory = $true)]
        [string]$Context
    )

    $encoding = [string](Get-RequiredPropertyValue -Object $Envelope -Name "encoding")
    if ($encoding -ne "hex") {
        throw "$Context uses unsupported byte encoding '$encoding'"
    }
    $hex = [string](Get-RequiredPropertyValue -Object $Envelope -Name "bytes")
    if ($hex -notmatch '^(?:[0-9A-Fa-f]{2})*$') {
        throw "$Context contains malformed hexadecimal bytes"
    }
    $byteLength = [int](Get-RequiredPropertyValue -Object $Envelope -Name "byte_length")
    if ($byteLength -lt 0 -or $hex.Length -ne $byteLength * 2) {
        throw "$Context byte_length does not match its hexadecimal payload"
    }
    return $hex.ToLowerInvariant()
}

function Convert-DoubleToHex {
    param(
        [Parameter(Mandatory = $true)]
        [double]$Value
    )

    $bytes = [System.BitConverter]::GetBytes($Value)
    if ([System.BitConverter]::IsLittleEndian) {
        [array]::Reverse($bytes)
    }
    return Convert-ByteArrayToHex -Bytes $bytes
}

function New-NormalizedInstruction {
    param(
        [Parameter(Mandatory = $true)]
        [int]$Pc,
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Line,
        [Parameter(Mandatory = $true)]
        [string]$Opcode,
        [Parameter(Mandatory = $true)]
        [int]$A,
        [AllowNull()][object]$B,
        [AllowNull()][object]$C,
        [AllowNull()][object]$Bx,
        [AllowNull()][object]$SBx
    )

    if (-not $opcodeSpecification.ContainsKey($Opcode)) {
        throw "unknown Lua 5.1 opcode '$Opcode'"
    }
    $spec = $opcodeSpecification[$Opcode]
    $word = [uint64]$spec.code -bor ([uint64]$A -shl 6)
    switch ($spec.mode) {
        "ABC" {
            if ($null -eq $B -or $null -eq $C) {
                throw "$Opcode requires B and C operands"
            }
            $word = $word -bor ([uint64][int]$C -shl 14) -bor ([uint64][int]$B -shl 23)
        }
        "ABx" {
            if ($null -eq $Bx) {
                throw "$Opcode requires Bx operand"
            }
            $word = $word -bor ([uint64][int]$Bx -shl 14)
        }
        "AsBx" {
            if ($null -eq $SBx) {
                throw "$Opcode requires sBx operand"
            }
            $encodedBx = [int]$SBx + 131071
            if ($encodedBx -lt 0 -or $encodedBx -gt 262143) {
                throw "$Opcode has out-of-range sBx operand $SBx"
            }
            $word = $word -bor ([uint64]$encodedBx -shl 14)
        }
    }
    $word = $word -band [uint64]4294967295
    $decodedA = [int](($word -shr 6) -band 0xff)
    $decodedC = [int](($word -shr 14) -band 0x1ff)
    $decodedB = [int](($word -shr 23) -band 0x1ff)
    $decodedBx = [int](($word -shr 14) -band 0x3ffff)
    $decodedSBx = $decodedBx - 131071

    $rkOperands = [ordered]@{}
    foreach ($operandName in $spec.rk) {
        $encoded = if ($operandName -eq "b") { $decodedB } else { $decodedC }
        $rkOperands[$operandName] = [pscustomobject][ordered]@{
            encoded = $encoded
            kind    = if (($encoded -band 0x100) -ne 0) { "constant" } else { "register" }
            index   = if (($encoded -band 0x100) -ne 0) { $encoded -band 0xff } else { $encoded }
        }
    }

    return [pscustomobject][ordered]@{
        pc         = $Pc
        line       = $Line
        opcode     = $Opcode
        rawWord    = $word
        rawWordHex = ('0x{0:x8}' -f $word)
        a          = $decodedA
        b          = $decodedB
        c          = $decodedC
        bx         = $decodedBx
        sbx        = $decodedSBx
        rkOperands = $rkOperands
    }
}

function Convert-CppConstant {
    param(
        [Parameter(Mandatory = $true)]
        [int]$Index,
        [Parameter(Mandatory = $true)]
        [string]$Text
    )

    $type = "unknown"
    $value = $Text
    $known = $false
    if ($Text -eq "nil") {
        $type = "nil"
        $value = $null
        $known = $true
    }
    elseif ($Text -match '^boolean (true|false)$') {
        $type = "boolean"
        $value = $Matches[1] -eq "true"
        $known = $true
    }
    elseif ($Text -match '^number (.+)$') {
        $type = "number"
        $number = 0.0
        if ([double]::TryParse(
            $Matches[1],
            [System.Globalization.NumberStyles]::Float,
            [System.Globalization.CultureInfo]::InvariantCulture,
            [ref]$number
        )) {
            $value = Convert-DoubleToHex -Value $number
            $known = $true
        }
    }
    elseif ($Text -match '^string "(.*)"$') {
        $type = "string"
        $value = Convert-CppEscapedStringToHex -Text $Matches[1]
        $known = $true
    }

    return [pscustomobject][ordered]@{
        index      = $Index
        type       = $type
        valueKnown = $known
        value      = $value
    }
}

function Convert-RustConstant {
    param(
        [Parameter(Mandatory = $true)]
        [int]$Index,
        [Parameter(Mandatory = $true)]
        [object]$Constant
    )

    $type = [string](Get-RequiredPropertyValue -Object $Constant -Name "type")
    $known = $false
    $value = $null
    switch ($type) {
        "nil" {
            $known = $true
        }
        "boolean" {
            $value = [bool](Get-RequiredPropertyValue -Object $Constant -Name "value")
            $known = $true
        }
        "number" {
            $bits = [string](Get-RequiredPropertyValue -Object $Constant -Name "bits")
            if ($bits -notmatch '^[0-9A-Fa-f]{16}$') {
                throw "Rust number constant $Index has malformed IEEE-754 bits"
            }
            $value = $bits.ToLowerInvariant()
            $known = $true
        }
        "string" {
            $envelope = Get-RequiredPropertyValue -Object $Constant -Name "value"
            if ($null -eq $envelope) {
                throw "Rust string constant $Index has null byte evidence"
            }
            $value = Convert-RustByteEnvelopeToHex -Envelope $envelope `
                -Context "Rust string constant $Index"
            $known = $true
        }
    }
    return [pscustomobject][ordered]@{
        index      = $Index
        type       = $type
        valueKnown = $known
        value      = $value
    }
}

function Get-BytecodeCoverage {
    param(
        [Parameter(Mandatory = $true)]
        [object[]]$Protos,
        [Parameter(Mandatory = $true)]
        [string]$Adapter
    )

    $instructions = @($Protos | ForEach-Object { @($_.instructions) })
    $constants = @($Protos | ForEach-Object { @($_.constants) })
    $metadataComplete = @($Protos | Where-Object {
        $null -eq $_.lineDefined -or $null -eq $_.lastLineDefined -or
        $null -eq $_.params -or $null -eq $_.varargFlags -or $null -eq $_.maxStack
    }).Count -eq 0
    $lineInfoComplete = @($instructions | Where-Object { $null -eq $_.line }).Count -eq 0
    $subProtoComplete = @($Protos | Where-Object { $null -eq $_.childCount }).Count -eq 0
    if ($subProtoComplete) {
        $declaredProtoCount = 1
        foreach ($proto in $Protos) {
            $declaredProtoCount += [int]$proto.childCount
        }
        $subProtoComplete = $declaredProtoCount -eq $Protos.Count
    }
    $localNamesComplete = $true
    $upvalueNamesComplete = $true
    foreach ($proto in $Protos) {
        if ($null -eq $proto.localNames) {
            $localNamesComplete = $false
        }
        else {
            foreach ($name in @($proto.localNames)) {
                if ($null -eq $name) {
                    $localNamesComplete = $false
                }
            }
        }
        if ($null -eq $proto.upvalueNames) {
            $upvalueNamesComplete = $false
        }
        else {
            foreach ($name in @($proto.upvalueNames)) {
                if ($null -eq $name) {
                    $upvalueNamesComplete = $false
                }
            }
        }
    }
    $constantOrderComplete = $true
    foreach ($proto in $Protos) {
        for ($index = 0; $index -lt @($proto.constants).Count; $index++) {
            if ($proto.constants[$index].index -ne $index) {
                $constantOrderComplete = $false
            }
        }
    }

    return [pscustomobject][ordered]@{
        opcodeSet38       = @($instructions | Where-Object {
            -not $opcodeSpecification.ContainsKey($_.opcode)
        }).Count -eq 0
        instructionWord32 = @($instructions | Where-Object { $null -eq $_.rawWord }).Count -eq 0
        decodedOperands   = @($instructions | Where-Object {
            $null -eq $_.a -or $null -eq $_.b -or $null -eq $_.c -or
            $null -eq $_.bx -or $null -eq $_.sbx
        }).Count -eq 0
        rkOperands        = @($instructions | Where-Object { $null -eq $_.rkOperands }).Count -eq 0
        constantValues    = @($constants | Where-Object { -not $_.valueKnown }).Count -eq 0
        constantOrder     = $constantOrderComplete
        subProtos         = $subProtoComplete
        functionMetadata  = $metadataComplete
        lineInfo          = $lineInfoComplete
        localNames        = $localNamesComplete
        upvalueNames      = $upvalueNamesComplete
    }
}

function Convert-CppBytecode {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Text
    )

    $protos = New-Object System.Collections.ArrayList
    $current = $null
    $pendingPath = $null
    $pendingProtoIndent = $null
    $protoStack = New-Object System.Collections.ArrayList
    foreach ($line in ($Text -split "\r?\n")) {
        $trimmed = $line.Trim()
        $indent = $line.Length - $line.TrimStart().Length
        if ($trimmed -match '^proto\[(\d+)\]\s+') {
            $parentIndent = $indent - 2
            $parent = @($protoStack | Where-Object { $_.indent -eq $parentIndent } | Select-Object -Last 1)
            if ($parent.Count -ne 1) {
                throw "unable to resolve parent Proto for child label '$trimmed'"
            }
            $pendingPath = "$($parent[0].data.path).children[$([int]$Matches[1])]"
            $pendingProtoIndent = $indent + 2
            continue
        }
        if ($trimmed -eq "Proto") {
            while ($protoStack.Count -gt 0 -and $protoStack[$protoStack.Count - 1].indent -ge $indent) {
                $protoStack.RemoveAt($protoStack.Count - 1)
            }
            $path = if ($null -ne $pendingPath -and $pendingProtoIndent -eq $indent) {
                $pendingPath
            }
            elseif ($protos.Count -eq 0 -and $indent -eq 0) {
                "0"
            }
            else {
                "orphan[$($protos.Count)]"
            }
            $pendingPath = $null
            $pendingProtoIndent = $null
            $current = [ordered]@{
                path            = $path
                lineDefined     = $null
                lastLineDefined = $null
                params          = $null
                varargFlags     = $null
                maxStack        = $null
                upvalueNames    = $null
                localNames      = $null
                childCount      = $null
                constants       = New-Object System.Collections.ArrayList
                instructions    = New-Object System.Collections.ArrayList
            }
            [void]$protos.Add($current)
            [void]$protoStack.Add([pscustomobject]@{ indent = $indent; data = $current })
            continue
        }
        if ($null -eq $current) {
            continue
        }

        if ($trimmed -match '^linedefined:\s*(-?\d+)$') {
            $current.lineDefined = [int]$Matches[1]
        }
        elseif ($trimmed -match '^lastlinedefined:\s*(-?\d+)$') {
            $current.lastLineDefined = [int]$Matches[1]
        }
        elseif ($trimmed -match '^numparams:\s*(\d+)$') {
            $current.params = [int]$Matches[1]
        }
        elseif ($trimmed -match '^is_vararg:\s*(?:true|false)\s+\(flags=(\d+)\)$') {
            $current.varargFlags = [int]$Matches[1]
        }
        elseif ($trimmed -match '^maxStackSize:\s*(\d+)$') {
            $current.maxStack = [int]$Matches[1]
        }
        elseif ($trimmed -match '^upvalues\s+\((\d+)\):\s*(.*)$') {
            $count = [int]$Matches[1]
            $names = $Matches[2]
            if ($count -eq 0) {
                $current.upvalueNames = [object[]]@()
            }
            else {
                $parsedNames = @($names -split ',\s*')
                if ($parsedNames.Count -ne $count) {
                    throw "C++ output declares $count upvalue names but prints $($parsedNames.Count)"
                }
                $current.upvalueNames = [object[]]@($parsedNames | ForEach-Object {
                    if ($_ -eq "?") {
                        $null
                    }
                    else {
                        Convert-Utf8TextToHex -Text $_
                    }
                })
            }
        }
        elseif ($trimmed -match '^child protos\s+\((\d+)\)$') {
            $current.childCount = [int]$Matches[1]
        }
        elseif ($trimmed -match '^K\[(\d+)\]\s*=\s*(.*)$') {
            [void]$current.constants.Add((Convert-CppConstant -Index ([int]$Matches[1]) -Text $Matches[2]))
        }
        elseif ($trimmed -match '^(\d+)\s+\|\s+line\s+(-?\d+)\s+\|\s+([A-Z]+)\s+\|\s+(.+?)(?:\s+;.*)?$') {
            $pc = [int]$Matches[1]
            $sourceLine = [int]$Matches[2]
            $opcode = $Matches[3]
            $operands = $Matches[4]
            $a = $null
            $b = $null
            $c = $null
            $bx = $null
            $sbx = $null
            if ($operands -match '(?:^|\s)A=(-?\d+)') { $a = [int]$Matches[1] }
            if ($operands -match '(?:^|\s)B=(-?\d+)') { $b = [int]$Matches[1] }
            if ($operands -match '(?:^|\s)C=(-?\d+)') { $c = [int]$Matches[1] }
            if ($operands -match '(?:^|\s)Bx=(-?\d+)') { $bx = [int]$Matches[1] }
            if ($operands -match '(?:^|\s)sBx=(-?\d+)') { $sbx = [int]$Matches[1] }
            if ($null -eq $a) {
                throw "unable to parse A operand at C++ bytecode pc $pc"
            }
            [void]$current.instructions.Add((New-NormalizedInstruction `
                -Pc $pc -Line $sourceLine -Opcode $opcode -A $a -B $b -C $c -Bx $bx -SBx $sbx))
        }
    }
    if ($protos.Count -eq 0) {
        throw "C++ output contains no Proto"
    }

    $normalized = @($protos | ForEach-Object {
        [pscustomobject][ordered]@{
            path            = $_.path
            lineDefined     = $_.lineDefined
            lastLineDefined = $_.lastLineDefined
            params          = $_.params
            varargFlags     = $_.varargFlags
            maxStack        = $_.maxStack
            upvalueNames    = @($_.upvalueNames)
            localNames      = $_.localNames
            childCount      = $_.childCount
            constants       = @($_.constants | Sort-Object index)
            instructions    = @($_.instructions | Sort-Object pc)
        }
    })
    return [pscustomobject][ordered]@{
        success  = $true
        error    = $null
        protos   = $normalized
        coverage = Get-BytecodeCoverage -Protos $normalized -Adapter "CppFullText"
    }
}

function Add-RustProto {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Proto,
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [System.Collections.ArrayList]$Protos
    )

    $constants = New-Object System.Collections.ArrayList
    $constantValues = @(Get-RequiredPropertyValue -Object $Proto -Name "constants")
    for ($index = 0; $index -lt $constantValues.Count; $index++) {
        [void]$constants.Add((Convert-RustConstant -Index $index -Constant $constantValues[$index]))
    }

    $lineInfoProperty = $Proto.PSObject.Properties["line_info"]
    if ($null -eq $lineInfoProperty) {
        throw "missing property 'line_info'"
    }
    $lineInfo = if ($null -eq $lineInfoProperty.Value) {
        $null
    }
    else {
        @($lineInfoProperty.Value)
    }

    $instructionValues = @(Get-RequiredPropertyValue -Object $Proto -Name "instructions")
    if ($null -ne $lineInfo -and $lineInfo.Count -ne $instructionValues.Count) {
        throw "Rust JSON line_info count does not match instruction count at Proto $Path"
    }
    $instructions = New-Object System.Collections.ArrayList
    for ($index = 0; $index -lt $instructionValues.Count; $index++) {
        $instruction = $instructionValues[$index]
        $expectedPc = [int](Get-RequiredPropertyValue -Object $instruction -Name "pc")
        if ($expectedPc -ne $index) {
            throw "Rust JSON has non-sequential pc $expectedPc at Proto $Path index $index"
        }
        $lineValue = Get-RequiredPropertyValue -Object $instruction -Name "line"
        $expectedLine = if ($null -eq $lineValue) { $null } else { [int]$lineValue }
        if ($null -ne $lineInfo) {
            $lineInfoValue = $lineInfo[$index]
            if (($null -eq $expectedLine) -ne ($null -eq $lineInfoValue) -or
                ($null -ne $expectedLine -and [int]$expectedLine -ne [int]$lineInfoValue)) {
                throw "Rust JSON line_info disagrees with instruction line at Proto $Path pc $expectedPc"
            }
        }
        $expectedOpcode = [string](Get-RequiredPropertyValue -Object $instruction -Name "op")
        $expectedA = [int](Get-RequiredPropertyValue -Object $instruction -Name "a")
        $expectedB = [int](Get-RequiredPropertyValue -Object $instruction -Name "b")
        $expectedC = [int](Get-RequiredPropertyValue -Object $instruction -Name "c")
        $expectedBx = [int](Get-RequiredPropertyValue -Object $instruction -Name "bx")
        $expectedSBx = [int](Get-RequiredPropertyValue -Object $instruction -Name "sbx")
        $normalizedInstruction = New-NormalizedInstruction `
            -Pc $expectedPc -Line $expectedLine -Opcode $expectedOpcode -A $expectedA `
            -B $expectedB -C $expectedC -Bx $expectedBx -SBx $expectedSBx
        foreach ($operandName in @("a", "b", "c", "bx", "sbx")) {
            $expectedValue = Get-Variable -Name "expected$($operandName.Substring(0, 1).ToUpperInvariant())$($operandName.Substring(1))" -ValueOnly
            if ([int]$normalizedInstruction.$operandName -ne [int]$expectedValue) {
                throw "Rust JSON has internally inconsistent $operandName at pc $expectedPc"
            }
        }
        [void]$instructions.Add($normalizedInstruction)
    }

    $upvalueNamesProperty = $Proto.PSObject.Properties["upvalue_names"]
    if ($null -eq $upvalueNamesProperty) {
        throw "missing property 'upvalue_names'"
    }
    $upvalueNames = $null
    if ($null -ne $upvalueNamesProperty.Value) {
        $upvalueNames = [object[]]@($upvalueNamesProperty.Value | ForEach-Object {
            if ($null -eq $_) {
                $null
            }
            else {
                Convert-RustByteEnvelopeToHex -Envelope $_ `
                    -Context "Rust upvalue name at Proto $Path"
            }
        })
    }

    $localNamesProperty = $Proto.PSObject.Properties["local_names"]
    if ($null -eq $localNamesProperty) {
        throw "missing property 'local_names'"
    }
    $localNames = $null
    if ($null -ne $localNamesProperty.Value) {
        $localNames = [object[]]@($localNamesProperty.Value | ForEach-Object {
            if ($null -eq $_) {
                $null
            }
            else {
                Convert-RustByteEnvelopeToHex -Envelope $_ `
                    -Context "Rust local name at Proto $Path"
            }
        })
    }

    $lineDefinedValue = Get-RequiredPropertyValue -Object $Proto -Name "line_defined"
    $lastLineDefinedValue = Get-RequiredPropertyValue -Object $Proto -Name "last_line_defined"
    $paramsValue = Get-RequiredPropertyValue -Object $Proto -Name "params"
    $varargValue = Get-RequiredPropertyValue -Object $Proto -Name "vararg"
    $maxStackValue = Get-RequiredPropertyValue -Object $Proto -Name "max_stack"
    $childCountValue = Get-RequiredPropertyValue -Object $Proto -Name "child_count"
    $childValues = @(Get-RequiredPropertyValue -Object $Proto -Name "sub_protos")
    if ($null -ne $childCountValue -and [int]$childCountValue -ne $childValues.Count) {
        throw "Rust JSON child_count does not match sub_protos at Proto $Path"
    }

    [void]$Protos.Add([pscustomobject][ordered]@{
        path            = $Path
        lineDefined     = if ($null -eq $lineDefinedValue) { $null } else { [int]$lineDefinedValue }
        lastLineDefined = if ($null -eq $lastLineDefinedValue) { $null } else { [int]$lastLineDefinedValue }
        params          = if ($null -eq $paramsValue) { $null } else { [int]$paramsValue }
        varargFlags     = if ($null -eq $varargValue) { $null } else { [int]$varargValue }
        maxStack        = if ($null -eq $maxStackValue) { $null } else { [int]$maxStackValue }
        upvalueNames    = $upvalueNames
        localNames      = $localNames
        childCount      = if ($null -eq $childCountValue) { $null } else { [int]$childCountValue }
        constants       = @($constants)
        instructions    = @($instructions)
    })

    for ($index = 0; $index -lt $childValues.Count; $index++) {
        Add-RustProto -Proto $childValues[$index] -Path "$Path.children[$index]" -Protos $Protos
    }
}

function Convert-RustBytecode {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Text
    )

    try {
        $json = $Text | ConvertFrom-Json -ErrorAction Stop
    }
    catch {
        throw "Rust output is not valid JSON: $($_.Exception.Message)"
    }

    $schemaVersion = [int](Get-RequiredPropertyValue -Object $json -Name "schema_version")
    if ($schemaVersion -ne 2) {
        throw "unsupported Rust bytecode JSON schema version $schemaVersion"
    }

    $normalized = New-Object System.Collections.ArrayList
    Add-RustProto -Proto $json -Path "0" -Protos $normalized
    return [pscustomobject][ordered]@{
        success  = $true
        error    = $null
        protos   = @($normalized)
        coverage = Get-BytecodeCoverage -Protos @($normalized) -Adapter "RustJson"
    }
}

function Convert-BytecodeOutput {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet("CppFullText", "RustJson")]
        [string]$Adapter,
        [Parameter(Mandatory = $true)]
        [string]$Text
    )

    try {
        if ($Adapter -eq "CppFullText") {
            return Convert-CppBytecode -Text $Text
        }
        return Convert-RustBytecode -Text $Text
    }
    catch {
        return [pscustomobject][ordered]@{
            success  = $false
            error    = $_.Exception.Message
            protos   = @()
            coverage = $null
        }
    }
}

function Add-BytecodeDifference {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [System.Collections.ArrayList]$List,
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [string]$Kind,
        [AllowNull()][object]$Left,
        [AllowNull()][object]$Right,
        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    [void]$List.Add([pscustomobject][ordered]@{
        path    = $Path
        kind    = $Kind
        left    = $Left
        right   = $Right
        message = $Message
    })
}

$runId = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssfffZ")
if (-not $CppBytecodeExe) {
    $CppBytecodeExe = Join-Path $projectRoot "..\lua_cpp\bin\lua_bytecode.exe"
}
if (-not $RustBytecodeExe) {
    $configuredTarget = Join-Path $projectRoot "target\x86_64-pc-windows-msvc\debug\lua_bytecode.exe"
    $hostTarget = Join-Path $projectRoot "target\debug\lua_bytecode.exe"
    $RustBytecodeExe = if (Test-Path -LiteralPath $configuredTarget -PathType Leaf) {
        $configuredTarget
    }
    else {
        $hostTarget
    }
}
if (-not $OutputDir) {
    $OutputDir = Join-Path $projectRoot "target\parity\bytecode"
}

$InputPath = Resolve-ParityPath -Path $InputPath -BasePath $projectRoot
$CppBytecodeExe = Resolve-ParityPath -Path $CppBytecodeExe -BasePath $projectRoot
$RustBytecodeExe = Resolve-ParityPath -Path $RustBytecodeExe -BasePath $projectRoot
$OutputDir = Resolve-ParityPath -Path $OutputDir -BasePath $projectRoot
if ($RepresentativeManifest) {
    $RepresentativeManifest = Resolve-ParityPath -Path $RepresentativeManifest -BasePath $projectRoot
}
if (-not $ResultPath) {
    $ResultPath = Join-Path $OutputDir "report.json"
}
else {
    $ResultPath = Resolve-ParityPath -Path $ResultPath -BasePath $projectRoot
}
$runDirectory = Join-Path (Join-Path $OutputDir "runs") $runId

$requiredEvidence = @(
    "opcodeSet38", "instructionWord32", "decodedOperands", "rkOperands",
    "constantValues", "constantOrder", "subProtos", "functionMetadata",
    "lineInfo", "localNames", "upvalueNames"
)
$caseResults = New-Object System.Collections.ArrayList
$preflightIssues = New-Object System.Collections.ArrayList
$report = [ordered]@{
    schemaVersion = 2
    runner        = "compare_bytecode"
    purpose       = if ($InfrastructureSelfTest) { "infrastructure-self-test" } else { "cross-language-parity" }
    status        = "running"
    generatedAt   = [DateTime]::UtcNow.ToString("o")
    projectRoot   = $projectRoot
    runDirectory  = $runDirectory
    resultPath    = $ResultPath
    corpus        = [ordered]@{
        input                  = $InputPath
        mode                   = $CorpusMode
        representativeCount    = $RepresentativeCount
        representativeManifest = if ($RepresentativeManifest) { $RepresentativeManifest } else { $null }
        selected               = @()
    }
    tools         = [ordered]@{
        cpp  = $CppBytecodeExe
        rust = $RustBytecodeExe
    }
    lua51OpcodeCount = $opcodeSpecification.Count
    requiredEvidence = $requiredEvidence
    preflightIssues  = $preflightIssues
    summary       = [ordered]@{
        selected               = 0
        passed                 = 0
        failed                 = 0
        infrastructureFailures = 0
        semanticFailures       = 0
    }
    cases         = $caseResults
}

try {
    New-ParityDirectory -Path $runDirectory
}
catch {
    [Console]::Error.WriteLine("Cannot create parity output directory '$runDirectory': $($_.Exception.Message)")
    exit 2
}

if (-not (Test-Path -LiteralPath $InputPath)) {
    [void]$preflightIssues.Add("input path not found: $InputPath")
}
if ($opcodeSpecification.Count -ne 38) {
    [void]$preflightIssues.Add("runner opcode specification must contain all 38 Lua 5.1 opcodes")
}

$leftExecutable = $CppBytecodeExe
$rightExecutable = $RustBytecodeExe
$leftAdapter = "CppFullText"
$rightAdapter = "RustJson"
if ($InfrastructureSelfTest) {
    if ($SelfTestTool -eq "Cpp") {
        $leftExecutable = $CppBytecodeExe
        $rightExecutable = $CppBytecodeExe
        $leftAdapter = "CppFullText"
        $rightAdapter = "CppFullText"
    }
    else {
        $leftExecutable = $RustBytecodeExe
        $rightExecutable = $RustBytecodeExe
        $leftAdapter = "RustJson"
        $rightAdapter = "RustJson"
    }
}
if (-not (Test-Path -LiteralPath $leftExecutable -PathType Leaf)) {
    [void]$preflightIssues.Add("left bytecode tool not found: $leftExecutable")
}
if (-not (Test-Path -LiteralPath $rightExecutable -PathType Leaf)) {
    [void]$preflightIssues.Add("right bytecode tool not found: $rightExecutable")
}

if ($preflightIssues.Count -gt 0) {
    $report.status = "infrastructure-failed"
    $report.summary.infrastructureFailures = $preflightIssues.Count
    Write-ParityJson -Path $ResultPath -Value $report
    if ($JsonOutput) {
        $report | ConvertTo-Json -Depth 64 -Compress | Write-Output
    }
    [Console]::Error.WriteLine(("Bytecode parity preflight failed. Report: {0}" -f $ResultPath))
    exit 2
}

try {
    $files = @(Select-ParityCorpus -InputPath $InputPath -Mode $CorpusMode `
        -RepresentativeCount $RepresentativeCount -RepresentativeManifest $RepresentativeManifest)
}
catch {
    [void]$preflightIssues.Add($_.Exception.Message)
    $report.status = "infrastructure-failed"
    $report.summary.infrastructureFailures = 1
    Write-ParityJson -Path $ResultPath -Value $report
    if ($JsonOutput) {
        $report | ConvertTo-Json -Depth 64 -Compress | Write-Output
    }
    [Console]::Error.WriteLine(("Bytecode corpus selection failed. Report: {0}" -f $ResultPath))
    exit 2
}

$corpusRoot = if ((Get-Item -LiteralPath $InputPath).PSIsContainer) {
    $InputPath
}
else {
    Split-Path -Parent $InputPath
}
$report.corpus.selected = @($files | ForEach-Object {
    Get-ParityRelativePath -Root $corpusRoot -Path $_.FullName
})
$report.summary.selected = $files.Count

foreach ($file in $files) {
    $relativePath = Get-ParityRelativePath -Root $corpusRoot -Path $file.FullName
    $caseId = Get-ParityCaseId -RelativePath $relativePath
    $caseDirectory = Join-Path $runDirectory $caseId
    New-ParityDirectory -Path $caseDirectory
    $toolInput = $file.FullName.Replace('\', '/')

    $leftArguments = if ($leftAdapter -eq "CppFullText") {
        @($toolInput, "full")
    }
    else {
        @($toolInput, "--format=json")
    }
    $rightArguments = if ($rightAdapter -eq "CppFullText") {
        @($toolInput, "full")
    }
    else {
        @($toolInput, "--format=json")
    }

    $leftRaw = Invoke-ParityProcess -Executable $leftExecutable -Arguments $leftArguments `
        -WorkingDirectory $projectRoot -TimeoutSeconds $TimeoutSeconds
    $rightRaw = Invoke-ParityProcess -Executable $rightExecutable -Arguments $rightArguments `
        -WorkingDirectory $projectRoot -TimeoutSeconds $TimeoutSeconds
    $leftExecution = Save-ParityExecution -Execution $leftRaw -CaseDirectory $caseDirectory -Side "left"
    $rightExecution = Save-ParityExecution -Execution $rightRaw -CaseDirectory $caseDirectory -Side "right"

    $differences = New-Object System.Collections.ArrayList
    $infrastructureFailure = $false
    foreach ($side in @(
        [pscustomobject]@{ name = "left"; execution = $leftExecution },
        [pscustomobject]@{ name = "right"; execution = $rightExecution }
    )) {
        if ($side.execution.startError) {
            $infrastructureFailure = $true
            Add-BytecodeDifference -List $differences -Path "$($side.name).process" -Kind "start-error" `
                -Left $side.execution.startError -Right $null -Message "tool could not be started"
        }
        if ($side.execution.timedOut) {
            $infrastructureFailure = $true
            Add-BytecodeDifference -List $differences -Path "$($side.name).process" -Kind "timeout" `
                -Left $TimeoutSeconds -Right $null -Message "tool exceeded timeout"
        }
        if ($null -ne $side.execution.exitCode -and $side.execution.exitCode -ne 0) {
            $infrastructureFailure = $true
            Add-BytecodeDifference -List $differences -Path "$($side.name).exitCode" -Kind "process-exit" `
                -Left $side.execution.exitCode -Right 0 -Message "bytecode tool exited unsuccessfully"
        }
    }

    $leftParsed = if (-not $leftExecution.startError -and -not $leftExecution.timedOut) {
        Convert-BytecodeOutput -Adapter $leftAdapter -Text $leftExecution.stdout
    }
    else {
        [pscustomobject]@{ success = $false; error = "process did not complete"; protos = @(); coverage = $null }
    }
    $rightParsed = if (-not $rightExecution.startError -and -not $rightExecution.timedOut) {
        Convert-BytecodeOutput -Adapter $rightAdapter -Text $rightExecution.stdout
    }
    else {
        [pscustomobject]@{ success = $false; error = "process did not complete"; protos = @(); coverage = $null }
    }
    if (-not $leftParsed.success) {
        $infrastructureFailure = $true
        Add-BytecodeDifference -List $differences -Path "left.output" -Kind "parse-error" `
            -Left $leftParsed.error -Right $null -Message "left bytecode output is not parseable"
    }
    if (-not $rightParsed.success) {
        $infrastructureFailure = $true
        Add-BytecodeDifference -List $differences -Path "right.output" -Kind "parse-error" `
            -Left $rightParsed.error -Right $null -Message "right bytecode output is not parseable"
    }

    if ($leftParsed.success -and $rightParsed.success) {
        if (-not $InfrastructureSelfTest) {
            foreach ($evidence in $requiredEvidence) {
                $leftCovered = [bool]$leftParsed.coverage.$evidence
                $rightCovered = [bool]$rightParsed.coverage.$evidence
                if (-not ($leftCovered -and $rightCovered)) {
                    Add-BytecodeDifference -List $differences -Path "evidence.$evidence" `
                        -Kind "missing-evidence" -Left $leftCovered -Right $rightCovered `
                        -Message "required bytecode evidence is unavailable on one or both sides"
                }
            }
        }

        $comparison = Compare-ParityValue -Left $leftParsed.protos -Right $rightParsed.protos `
            -Path '$.protos' -MaximumDifferences 500
        foreach ($difference in $comparison.items) {
            [void]$differences.Add($difference)
        }
        if ($comparison.truncated) {
            Add-BytecodeDifference -List $differences -Path '$.protos' -Kind "truncated" `
                -Left 500 -Right $null -Message "difference list reached its configured limit"
        }
    }

    Write-ParityJson -Path (Join-Path $caseDirectory "left.normalized.json") -Value $leftParsed
    Write-ParityJson -Path (Join-Path $caseDirectory "right.normalized.json") -Value $rightParsed
    $caseStatus = if ($differences.Count -eq 0) { "passed" } else { "failed" }
    $sourceCopy = $null
    if ($caseStatus -eq "failed") {
        $sourceCopy = Join-Path $caseDirectory "input.lua"
        Copy-Item -LiteralPath $file.FullName -Destination $sourceCopy -Force
    }

    $caseResult = [pscustomobject][ordered]@{
        id                    = $caseId
        input                 = $file.FullName
        relativeInput         = $relativePath
        inputSha256           = Get-ParityFileSha256 -Path $file.FullName
        sourceCopy            = $sourceCopy
        status                = $caseStatus
        infrastructureFailure = $infrastructureFailure
        evidence              = [ordered]@{
            left  = $leftParsed.coverage
            right = $rightParsed.coverage
        }
        executions            = [ordered]@{
            left  = $leftExecution
            right = $rightExecution
        }
        differences           = @($differences)
        differenceCount       = $differences.Count
        artifact              = Join-Path $caseDirectory "case.json"
    }
    Write-ParityJson -Path $caseResult.artifact -Value $caseResult
    [void]$caseResults.Add($caseResult)

    if ($caseStatus -eq "passed") {
        $report.summary.passed++
        Write-Host "[PASS] $relativePath"
    }
    else {
        $report.summary.failed++
        if ($infrastructureFailure) {
            $report.summary.infrastructureFailures++
        }
        else {
            $report.summary.semanticFailures++
        }
        Write-Host "[FAIL] $relativePath -> $($caseResult.artifact)" -ForegroundColor Red
    }
}

if ($report.summary.failed -eq 0) {
    $report.status = if ($InfrastructureSelfTest) { "self-test-passed" } else { "passed" }
}
elseif ($report.summary.infrastructureFailures -gt 0) {
    $report.status = "infrastructure-failed"
}
else {
    $report.status = "differences-found"
}
$report.completedAt = [DateTime]::UtcNow.ToString("o")
$report.selfTestNotice = if ($InfrastructureSelfTest) {
    "This result proves runner determinism only; it is not a lua_cpp/lua_rust parity result."
}
else {
    $null
}
Write-ParityJson -Path $ResultPath -Value $report

Write-Host ""
Write-Host "Bytecode parity: $($report.status)"
Write-Host "  selected=$($report.summary.selected) passed=$($report.summary.passed) failed=$($report.summary.failed)"
Write-Host "  report=$ResultPath"
if ($JsonOutput) {
    $report | ConvertTo-Json -Depth 64 -Compress | Write-Output
}

if ($report.summary.infrastructureFailures -gt 0) {
    exit 2
}
if ($report.summary.failed -gt 0) {
    exit 1
}
exit 0
