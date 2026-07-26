param(
    [string]$Root = "",
    [string]$CandidateLua = "",
    [string]$OfficialLua = "",
    [string]$CppLua = "",
    [string]$CppRoot = "",
    [string]$CasesPath = "tests/compatibility/lua51-differential-cases.json",
    [string]$VersionProbePath = "tests/compatibility/lua51-version-probe.lua",
    [string]$DeviationLogPath = "docs/rust_migration/deviation_log.md",
    [ValidateSet("official-lua51", "cpp-87c15e6")]
    [string[]]$Lane = @("official-lua51", "cpp-87c15e6"),
    [ValidateRange(1, 3600)]
    [int]$TimeoutSeconds = 30,
    [string]$ResultPath = "target/compatibility/lua51-differential.json",
    [switch]$ComparatorSelfTestOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
} else {
    $Root = (Resolve-Path -LiteralPath $Root).Path
}

function Resolve-RootedPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [switch]$MustExist
    )

    $resolved = if ([System.IO.Path]::IsPathRooted($Path)) {
        [System.IO.Path]::GetFullPath($Path)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $Root $Path))
    }
    if ($MustExist -and -not (Test-Path -LiteralPath $resolved)) {
        throw "Required path does not exist: $resolved"
    }
    return $resolved
}

function Resolve-Executable {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Value,
        [Parameter(Mandatory = $true)]
        [string]$Role
    )

    if ([string]::IsNullOrWhiteSpace($Value)) {
        throw "No executable configured for $Role"
    }
    if ([System.IO.Path]::IsPathRooted($Value) -or
        $Value.Contains([System.IO.Path]::DirectorySeparatorChar) -or
        $Value.Contains([System.IO.Path]::AltDirectorySeparatorChar)) {
        $path = Resolve-RootedPath -Path $Value
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Missing $Role executable: $path"
        }
        return $path
    }

    $command = Get-Command $Value -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($null -eq $command) {
        throw "Could not find $Role executable on PATH: $Value"
    }
    return $command.Source
}

function Get-ByteSha256 {
    param([byte[]]$Bytes)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([System.BitConverter]::ToString(
            $sha.ComputeHash($Bytes)
        )).Replace("-", "").ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function ConvertTo-StreamEvidence {
    param([byte[]]$Bytes)
    return [ordered]@{
        byteLength = $Bytes.Length
        sha256 = Get-ByteSha256 $Bytes
        base64 = [System.Convert]::ToBase64String($Bytes)
        utf8 = [System.Text.Encoding]::UTF8.GetString($Bytes)
    }
}

function Get-ObservedBytes {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Evidence
    )

    try {
        [byte[]]$bytes = [System.Convert]::FromBase64String(
            [string]$Evidence.base64
        )
    } catch {
        throw "Invalid observation base64: $($_.Exception.Message)"
    }
    if ($bytes.Length -ne [int]$Evidence.byteLength) {
        throw (
            "Observation byteLength mismatch: metadata={0}, decoded={1}" -f
            $Evidence.byteLength,
            $bytes.Length
        )
    }
    $actualSha256 = Get-ByteSha256 $bytes
    if ($actualSha256 -cne [string]$Evidence.sha256) {
        throw (
            "Observation sha256 mismatch: metadata={0}, decoded={1}" -f
            $Evidence.sha256,
            $actualSha256
        )
    }
    return ,$bytes
}

function Normalize-ObservedBytes {
    param(
        [byte[]]$Bytes,
        [bool]$NormalizeLineEndings
    )
    if (-not $NormalizeLineEndings) {
        return ,([byte[]]$Bytes.Clone())
    }

    $normalized = [System.Collections.Generic.List[byte]]::new($Bytes.Length)
    for ($index = 0; $index -lt $Bytes.Length; $index++) {
        $byte = $Bytes[$index]
        if ($byte -eq 0x0d) {
            $normalized.Add(0x0a)
            if ($index + 1 -lt $Bytes.Length -and
                $Bytes[$index + 1] -eq 0x0a) {
                $index++
            }
        } else {
            $normalized.Add($byte)
        }
    }
    return ,$normalized.ToArray()
}

function Test-ByteArrayEqual {
    param(
        [byte[]]$Left,
        [byte[]]$Right
    )
    if ($Left.Length -ne $Right.Length) {
        return $false
    }
    for ($index = 0; $index -lt $Left.Length; $index++) {
        if ($Left[$index] -ne $Right[$index]) {
            return $false
        }
    }
    return $true
}

function ConvertTo-ComparableEvidence {
    param([byte[]]$Bytes)
    return [ordered]@{
        byteLength = $Bytes.Length
        sha256 = Get-ByteSha256 $Bytes
        base64 = [System.Convert]::ToBase64String($Bytes)
    }
}

function Get-AsciiEvidenceText {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Evidence,
        [bool]$NormalizeLineEndings
    )
    [byte[]]$bytes = Get-ObservedBytes -Evidence $Evidence
    [byte[]]$comparable = Normalize-ObservedBytes `
        -Bytes $bytes `
        -NormalizeLineEndings $NormalizeLineEndings
    foreach ($byte in $comparable) {
        if ($byte -gt 0x7f) {
            throw "Semantic evidence requested ASCII decoding for non-ASCII bytes"
        }
    }
    return [System.Text.Encoding]::ASCII.GetString($comparable)
}

function Test-ObjectProperty {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Object,
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    if ($Object -is [System.Collections.IDictionary]) {
        return $Object.Contains($Name)
    }
    return $Object.PSObject.Properties.Name -contains $Name
}

function ConvertFrom-DeviationRegistryText {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Text
    )

    $registry = [System.Collections.Generic.Dictionary[string, string]]::new(
        [System.StringComparer]::Ordinal
    )
    $inRegistry = $false
    $foundRegistry = $false
    foreach ($rawLine in ($Text -split '\r?\n')) {
        $line = $rawLine.Trim()
        if (-not $inRegistry) {
            if ($line -ceq "## Registry") {
                $inRegistry = $true
                $foundRegistry = $true
            }
            continue
        }
        if ($line -match '^##\s+') {
            break
        }
        if ($line -notmatch '^\|\s*NOTE-[^|]*\|') {
            continue
        }

        $columns = @($line.Trim("|") -split '\|' | ForEach-Object {
            $_.Trim()
        })
        if ($columns.Count -ne 5 -or
            $columns[0] -notmatch '^NOTE-\d{3}$') {
            throw "Malformed deviation registry row: $line"
        }
        $id = $columns[0]
        $status = $columns[4].Trim().Trim('`')
        if ([string]::IsNullOrWhiteSpace($status)) {
            throw "Deviation registry entry '$id' has an empty status"
        }
        if ($registry.ContainsKey($id)) {
            throw "Duplicate deviation registry entry: $id"
        }
        $registry.Add($id, $status)
    }
    if (-not $foundRegistry) {
        throw "Deviation log lacks the '## Registry' section"
    }
    if ($registry.Count -eq 0) {
        throw "Deviation registry contains no NOTE entries"
    }
    return ,$registry
}

function Read-DeviationRegistry {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Deviation log does not exist: $Path"
    }
    $text = Get-Content -LiteralPath $Path -Raw
    return ,(ConvertFrom-DeviationRegistryText -Text $text)
}

function Get-ExpectedDifferenceOptional {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Expected
    )

    if (-not (Test-ObjectProperty -Object $Expected -Name "optional")) {
        return $false
    }
    if ($Expected.optional -isnot [bool]) {
        throw "Expected difference '$($Expected.id)' has a non-boolean optional value"
    }
    return [bool]$Expected.optional
}

function Test-DifferentialManifestDefinition {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Manifest,
        [Parameter(Mandatory = $true)]
        [System.Collections.Generic.Dictionary[string, string]]
        $DeviationRegistry
    )

    if (-not (Test-ObjectProperty -Object $Manifest -Name "cases")) {
        throw "Differential manifest lacks cases"
    }
    $cases = @($Manifest.cases)
    if ($cases.Count -eq 0) {
        throw "Differential manifest must contain at least one case"
    }

    $caseIds = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::Ordinal
    )
    foreach ($case in $cases) {
        if (-not (Test-ObjectProperty -Object $case -Name "id") -or
            [string]::IsNullOrWhiteSpace([string]$case.id)) {
            throw "Differential case has an empty id"
        }
        if (-not $caseIds.Add([string]$case.id)) {
            throw "Duplicate differential case id: $($case.id)"
        }
        if (-not (Test-ObjectProperty -Object $case -Name "script") -or
            [string]::IsNullOrWhiteSpace([string]$case.script)) {
            throw "Differential case '$($case.id)' lacks a script"
        }
    }

    if (-not (
        Test-ObjectProperty -Object $Manifest -Name "expectedDifferences"
    )) {
        throw "Differential manifest lacks expectedDifferences"
    }
    $expectedIds = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::Ordinal
    )
    $validLanes = @("official-lua51", "cpp-87c15e6")
    $validFields = @("outcome", "exitStatus", "stdout", "stderr")
    foreach ($expected in @($Manifest.expectedDifferences)) {
        if (-not (Test-ObjectProperty -Object $expected -Name "id") -or
            [string]::IsNullOrWhiteSpace([string]$expected.id)) {
            throw "Expected difference has an empty id"
        }
        if (-not $expectedIds.Add([string]$expected.id)) {
            throw "Duplicate expected difference id: $($expected.id)"
        }
        if (-not (Test-ObjectProperty -Object $expected -Name "deviation") -or
            [string]::IsNullOrWhiteSpace([string]$expected.deviation)) {
            throw "Expected difference '$($expected.id)' lacks a deviation id"
        }
        $deviationId = [string]$expected.deviation
        if (-not $DeviationRegistry.ContainsKey($deviationId)) {
            throw (
                "Expected difference '$($expected.id)' references unknown " +
                "deviation: $deviationId"
            )
        }
        $deviationStatus = $DeviationRegistry[$deviationId]
        if ($deviationStatus -cne "approved") {
            throw (
                "Expected difference '$($expected.id)' references deviation " +
                "'$deviationId' with non-approved status '$deviationStatus'"
            )
        }
        if (-not (Test-ObjectProperty -Object $expected -Name "lane") -or
            [string]$expected.lane -notin $validLanes) {
            throw (
                "Expected difference '$($expected.id)' references invalid lane: " +
                "$($expected.lane)"
            )
        }
        $null = Get-ExpectedDifferenceOptional -Expected $expected

        $hasProbe = Test-ObjectProperty -Object $expected -Name "probe"
        $hasCase = Test-ObjectProperty -Object $expected -Name "case"
        if ($hasProbe -eq $hasCase) {
            throw (
                "Expected difference '$($expected.id)' must reference exactly " +
                "one probe or case"
            )
        }
        if ($hasProbe -and [string]$expected.probe -cne "_VERSION") {
            throw (
                "Expected difference '$($expected.id)' references unsupported " +
                "probe: $($expected.probe)"
            )
        }
        if ($hasCase -and -not $caseIds.Contains([string]$expected.case)) {
            throw (
                "Expected difference '$($expected.id)' references unknown case: " +
                "$($expected.case)"
            )
        }
        if ($hasCase -and (
            -not (Test-ObjectProperty -Object $expected -Name "reason") -or
            [string]::IsNullOrWhiteSpace([string]$expected.reason)
        )) {
            throw "Case expected difference '$($expected.id)' lacks a reason"
        }

        if (-not (Test-ObjectProperty -Object $expected -Name "fields")) {
            throw "Expected difference '$($expected.id)' lacks fields"
        }
        $fields = @($expected.fields)
        if ($fields.Count -eq 0) {
            throw "Expected difference '$($expected.id)' has no fields"
        }
        $fieldIds = [System.Collections.Generic.HashSet[string]]::new(
            [System.StringComparer]::Ordinal
        )
        foreach ($field in $fields) {
            if ([string]$field -notin $validFields) {
                throw (
                    "Expected difference '$($expected.id)' references invalid " +
                    "field: $field"
                )
            }
            if (-not $fieldIds.Add([string]$field)) {
                throw (
                    "Expected difference '$($expected.id)' repeats field: $field"
                )
            }
        }

        if ($hasProbe) {
            if (($fields -join ",") -cne "stdout") {
                throw (
                    "Version expected difference '$($expected.id)' must match " +
                    "exactly the stdout field"
                )
            }
            foreach ($property in @("referenceUtf8", "candidateUtf8")) {
                if (-not (
                    Test-ObjectProperty -Object $expected -Name $property
                )) {
                    throw (
                        "Version expected difference '$($expected.id)' lacks " +
                        $property
                    )
                }
            }
        } else {
            foreach ($field in $fields) {
                $properties = switch ($field) {
                    "stdout" {
                        @(
                            "referenceBase64",
                            "candidateBase64",
                            "referenceSha256",
                            "candidateSha256"
                        )
                    }
                    "stderr" {
                        @(
                            "referenceBase64",
                            "candidateBase64",
                            "referenceSha256",
                            "candidateSha256"
                        )
                    }
                    "exitStatus" {
                        @("referenceExitStatus", "candidateExitStatus")
                    }
                    "outcome" {
                        @("referenceOutcome", "candidateOutcome")
                    }
                }
                foreach ($property in $properties) {
                    if (-not (
                        Test-ObjectProperty -Object $expected -Name $property
                    )) {
                        throw (
                            "Expected difference '$($expected.id)' lacks " +
                            "$property for $field"
                        )
                    }
                }
            }
        }
    }
}

function Get-ObservationInfrastructureFailure {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Context,
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Observation
    )

    if ($Observation.outcome -ceq "completed") {
        return ""
    }
    $detail = if ([string]::IsNullOrWhiteSpace(
        [string]$Observation.infrastructureError
    )) {
        ""
    } else {
        ": $($Observation.infrastructureError)"
    }
    return "$Context outcome was '$($Observation.outcome)'$detail"
}

function Get-ExpectedDifferenceConsumptionFailures {
    param(
        [Parameter(Mandatory = $true)]
        [object[]]$ExpectedDifferences,
        [Parameter(Mandatory = $true)]
        [System.Collections.Generic.Dictionary[string, int]]$Consumption,
        [Parameter(Mandatory = $true)]
        [string[]]$RequestedLanes
    )

    $failures = [System.Collections.Generic.List[string]]::new()
    foreach ($expected in $ExpectedDifferences) {
        if (@($RequestedLanes | Where-Object {
            $_ -ceq [string]$expected.lane
        }).Count -eq 0) {
            continue
        }
        $count = $Consumption[[string]$expected.id]
        $optional = Get-ExpectedDifferenceOptional -Expected $expected
        if ($count -gt 1) {
            $failures.Add(
                "Expected difference '$($expected.id)' was consumed $count times"
            )
        } elseif ($count -eq 0 -and -not $optional) {
            $failures.Add(
                "Required expected difference '$($expected.id)' was not consumed"
            )
        }
    }
    return @($failures)
}

function Invoke-ObservedProcess {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Executable,
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments,
        [Parameter(Mandatory = $true)]
        [int]$Timeout
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    if ($startInfo.PSObject.Properties.Name -contains "ArgumentList") {
        foreach ($argument in $Arguments) {
            $null = $startInfo.ArgumentList.Add($argument)
        }
    } else {
        $startInfo.Arguments = ($Arguments | ForEach-Object {
            '"' + ($_ -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
        }) -join " "
    }
    $startInfo.WorkingDirectory = $Root
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $stdoutMemory = [System.IO.MemoryStream]::new()
    $stderrMemory = [System.IO.MemoryStream]::new()
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        try {
            if (-not $process.Start()) {
                throw "Process.Start returned false"
            }
        } catch {
            $stopwatch.Stop()
            return [ordered]@{
                outcome = "infrastructure-error"
                exitStatus = $null
                durationMs = $stopwatch.ElapsedMilliseconds
                stdout = ConvertTo-StreamEvidence ([byte[]]@())
                stderr = ConvertTo-StreamEvidence ([byte[]]@())
                infrastructureError = $_.Exception.Message
            }
        }

        $stdoutTask = $process.StandardOutput.BaseStream.CopyToAsync($stdoutMemory)
        $stderrTask = $process.StandardError.BaseStream.CopyToAsync($stderrMemory)
        $completed = $process.WaitForExit($Timeout * 1000)
        if (-not $completed) {
            try {
                $process.Kill($true)
            } catch {
                Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            }
            $null = $process.WaitForExit()
        }
        $null = $stdoutTask.GetAwaiter().GetResult()
        $null = $stderrTask.GetAwaiter().GetResult()
        $stopwatch.Stop()

        return [ordered]@{
            outcome = if ($completed) { "completed" } else { "timeout" }
            exitStatus = if ($completed) { $process.ExitCode } else { $null }
            durationMs = $stopwatch.ElapsedMilliseconds
            stdout = ConvertTo-StreamEvidence $stdoutMemory.ToArray()
            stderr = ConvertTo-StreamEvidence $stderrMemory.ToArray()
            infrastructureError = $null
        }
    } finally {
        $stopwatch.Stop()
        $stdoutMemory.Dispose()
        $stderrMemory.Dispose()
        $process.Dispose()
    }
}

function Get-SemanticEvidence {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CaseId,
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Observation
    )

    if ($CaseId -notin @("value-types", "error-category", "gc-weak-value")) {
        return [ordered]@{}
    }
    $stdout = Get-AsciiEvidenceText `
        -Evidence $Observation.stdout `
        -NormalizeLineEndings $true
    switch ($CaseId) {
        "value-types" {
            return [ordered]@{
                returnValueTypes = @($stdout.TrimEnd("`n") -split "`n")
            }
        }
        "error-category" {
            $fields = @($stdout.Trim() -split ":")
            return [ordered]@{
                protectedCallSucceeded = if ($fields.Count -ge 1) { $fields[0] } else { "" }
                errorValueType = if ($fields.Count -ge 2) { $fields[1] } else { "" }
                errorCategory = if ($fields.Count -ge 3) { $fields[2] } else { "" }
            }
        }
        "gc-weak-value" {
            return [ordered]@{
                gcObservableSideEffect = $stdout.Trim()
            }
        }
        default {
            return [ordered]@{}
        }
    }
}

function Compare-Observations {
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Reference,
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Candidate,
        [bool]$NormalizeStdout = $false,
        [bool]$NormalizeStderr = $false
    )

    $mismatches = [System.Collections.Generic.List[object]]::new()
    if ($Reference.outcome -ne $Candidate.outcome) {
        $mismatches.Add([ordered]@{
            field = "outcome"
            reference = $Reference.outcome
            candidate = $Candidate.outcome
        })
    }
    if ($Reference.exitStatus -ne $Candidate.exitStatus) {
        $mismatches.Add([ordered]@{
            field = "exitStatus"
            reference = $Reference.exitStatus
            candidate = $Candidate.exitStatus
        })
    }
    foreach ($channel in @("stdout", "stderr")) {
        $normalizeLineEndings = if ($channel -eq "stdout") {
            $NormalizeStdout
        } else {
            $NormalizeStderr
        }
        [byte[]]$referenceBytes = Get-ObservedBytes `
            -Evidence $Reference[$channel]
        [byte[]]$candidateBytes = Get-ObservedBytes `
            -Evidence $Candidate[$channel]
        [byte[]]$referenceComparable = Normalize-ObservedBytes `
            -Bytes $referenceBytes `
            -NormalizeLineEndings $normalizeLineEndings
        [byte[]]$candidateComparable = Normalize-ObservedBytes `
            -Bytes $candidateBytes `
            -NormalizeLineEndings $normalizeLineEndings
        if (-not (Test-ByteArrayEqual `
            -Left $referenceComparable `
            -Right $candidateComparable)) {
            $referenceNormalized = ConvertTo-ComparableEvidence $referenceComparable
            $candidateNormalized = ConvertTo-ComparableEvidence $candidateComparable
            $mismatches.Add([ordered]@{
                field = $channel
                referenceSha256 = $Reference[$channel].sha256
                candidateSha256 = $Candidate[$channel].sha256
                referenceBase64 = $Reference[$channel].base64
                candidateBase64 = $Candidate[$channel].base64
                referenceComparableSha256 = $referenceNormalized.sha256
                candidateComparableSha256 = $candidateNormalized.sha256
                referenceComparableBase64 = $referenceNormalized.base64
                candidateComparableBase64 = $candidateNormalized.base64
                referenceUtf8 = $Reference[$channel].utf8
                candidateUtf8 = $Candidate[$channel].utf8
            })
        }
    }
    return @($mismatches)
}

function New-ComparatorSelfTestObservation {
    param(
        [Parameter(Mandatory = $true)]
        [string]$StdoutBase64,
        [Parameter(Mandatory = $true)]
        [string]$StdoutSha256,
        [Parameter(Mandatory = $true)]
        [int]$StdoutByteLength,
        [Parameter(Mandatory = $true)]
        [string]$StdoutUtf8
    )
    return [ordered]@{
        outcome = "completed"
        exitStatus = 0
        stdout = [ordered]@{
            byteLength = $StdoutByteLength
            sha256 = $StdoutSha256
            base64 = $StdoutBase64
            utf8 = $StdoutUtf8
        }
        stderr = [ordered]@{
            byteLength = 0
            sha256 = "e3b0c44298fc1c149afbf4c8996fb924" +
                "27ae41e4649b934ca495991b7852b855"
            base64 = ""
            utf8 = ""
        }
    }
}

function Test-ByteComparatorInfrastructure {
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.Generic.Dictionary[string, string]]
        $DeviationRegistry
    )

    foreach ($approvedId in @("NOTE-001", "NOTE-011")) {
        if (-not $DeviationRegistry.ContainsKey($approvedId) -or
            $DeviationRegistry[$approvedId] -cne "approved") {
            throw "Comparator self-test requires approved registry entry $approvedId"
        }
    }
    if (-not $DeviationRegistry.ContainsKey("NOTE-002") -or
        $DeviationRegistry["NOTE-002"] -ceq "approved") {
        throw "Comparator self-test requires a non-approved NOTE-002 registry entry"
    }

    $replacementDisplay = [System.Text.Encoding]::UTF8.GetString(
        [byte[]](0xff, 0x00, 0x80)
    )
    $invalidFf = New-ComparatorSelfTestObservation `
        -StdoutBase64 "/wCA" `
        -StdoutSha256 "ef192b7af54e943f206ab27075ec1805384c972c9959fc5820f1fa7d5268fcef" `
        -StdoutByteLength 3 `
        -StdoutUtf8 $replacementDisplay
    $invalidFe = New-ComparatorSelfTestObservation `
        -StdoutBase64 "/gCA" `
        -StdoutSha256 "b5b5a559dc6779a04a4209a3420506ab9e190071446d906feea6718fde814a6b" `
        -StdoutByteLength 3 `
        -StdoutUtf8 $replacementDisplay
    $invalidMismatch = @(Compare-Observations `
        -Reference $invalidFf `
        -Candidate $invalidFe)
    if ($invalidMismatch.Count -ne 1 -or
        $invalidMismatch[0].field -ne "stdout") {
        throw "Byte comparator self-test collapsed distinct invalid UTF-8 bytes"
    }

    $mixedEol = New-ComparatorSelfTestObservation `
        -StdoutBase64 "QQ0KQg1DCv8=" `
        -StdoutSha256 "14dd4c3f86221c6ae875982fc9e78878d595624a35d911a20b08d223e496a114" `
        -StdoutByteLength 8 `
        -StdoutUtf8 "diagnostic-only"
    $lfOnly = New-ComparatorSelfTestObservation `
        -StdoutBase64 "QQpCCkMK/w==" `
        -StdoutSha256 "49809435d2a0732d206decac3ab32cd18b93ab59b4d0146e22ba5b1b0126ad2e" `
        -StdoutByteLength 7 `
        -StdoutUtf8 "diagnostic-only"
    if (@(Compare-Observations `
        -Reference $mixedEol `
        -Candidate $lfOnly `
        -NormalizeStdout $true).Count -ne 0) {
        throw "Byte comparator self-test failed CRLF/CR byte normalization"
    }
    if (@(Compare-Observations `
        -Reference $mixedEol `
        -Candidate $lfOnly `
        -NormalizeStdout $false).Count -ne 1) {
        throw "Byte comparator self-test normalized bytes when disabled"
    }

    [byte[]]$normalized = Normalize-ObservedBytes `
        -Bytes ([System.Convert]::FromBase64String("QQ0KQg1DCv8=")) `
        -NormalizeLineEndings $true
    $normalizedEvidence = ConvertTo-ComparableEvidence $normalized
    if ($normalizedEvidence.base64 -cne "QQpCCkMK/w==" -or
        $normalizedEvidence.sha256 -cne
            "49809435d2a0732d206decac3ab32cd18b93ab59b4d0146e22ba5b1b0126ad2e") {
        throw "Byte comparator self-test did not match independent normalized vector"
    }

    $timeoutObservation = [ordered]@{
        outcome = "timeout"
        exitStatus = $null
        stdout = ConvertTo-StreamEvidence ([byte[]]@())
        stderr = ConvertTo-StreamEvidence ([byte[]]@())
        infrastructureError = $null
    }
    if (@(Compare-Observations `
        -Reference $timeoutObservation `
        -Candidate $timeoutObservation).Count -ne 0) {
        throw "Comparator timeout self-test vector unexpectedly differs"
    }
    $timeoutFailure = Get-ObservationInfrastructureFailure `
        -Context "self-test/reference" `
        -Observation $timeoutObservation
    if ([string]::IsNullOrWhiteSpace($timeoutFailure)) {
        throw "Completion guard self-test accepted matching timeouts"
    }

    $emptyManifestRejected = $false
    try {
        Test-DifferentialManifestDefinition -Manifest ([ordered]@{
            cases = @()
            expectedDifferences = @()
        }) -DeviationRegistry $DeviationRegistry
    } catch {
        $emptyManifestRejected =
            $_.Exception.Message -match "at least one case"
    }
    if (-not $emptyManifestRejected) {
        throw "Manifest self-test accepted an empty case list"
    }

    $requiredDifference = [ordered]@{
        id = "required-self-test-deviation"
        deviation = "NOTE-001"
        lane = "official-lua51"
        case = "self-test-case"
        reason = "self-test"
        fields = @("stdout")
        referenceBase64 = "QQ=="
        candidateBase64 = "Qg=="
        referenceSha256 =
            "559aead08264d5795d3909718cdd05abd49572e84fe55590eef31a88a08fdffd"
        candidateSha256 =
            "df7e70e5021544f4834bbee64a9e3789febc4be81470df629cad6ddb03320a5c"
    }
    $selfTestManifest = [ordered]@{
        cases = @(
            [ordered]@{
                id = "self-test-case"
                script = "self-test.lua"
            }
        )
        expectedDifferences = @($requiredDifference)
    }
    Test-DifferentialManifestDefinition `
        -Manifest $selfTestManifest `
        -DeviationRegistry $DeviationRegistry
    $zeroConsumption =
        [System.Collections.Generic.Dictionary[string, int]]::new(
            [System.StringComparer]::Ordinal
        )
    $zeroConsumption.Add([string]$requiredDifference.id, 0)
    $consumptionFailures = @(
        Get-ExpectedDifferenceConsumptionFailures `
            -ExpectedDifferences @($requiredDifference) `
            -Consumption $zeroConsumption `
            -RequestedLanes @("official-lua51")
    )
    if ($consumptionFailures.Count -ne 1 -or
        $consumptionFailures[0] -notmatch "was not consumed") {
        throw "Expected-difference self-test accepted an unconsumed requirement"
    }
    $optionalDifference = [ordered]@{}
    foreach ($key in $requiredDifference.Keys) {
        $optionalDifference[$key] = $requiredDifference[$key]
    }
    $optionalDifference.id = "optional-self-test-deviation"
    $optionalDifference.optional = $true
    $optionalConsumption =
        [System.Collections.Generic.Dictionary[string, int]]::new(
            [System.StringComparer]::Ordinal
        )
    $optionalConsumption.Add([string]$optionalDifference.id, 0)
    if (@(
        Get-ExpectedDifferenceConsumptionFailures `
            -ExpectedDifferences @($optionalDifference) `
            -Consumption $optionalConsumption `
            -RequestedLanes @("official-lua51")
    ).Count -ne 0) {
        throw "Expected-difference self-test rejected an optional unused entry"
    }

    foreach ($rejectedDeviation in @("NOTE-002", "NOTE-999")) {
        $rejectedDifference = [ordered]@{}
        foreach ($key in $requiredDifference.Keys) {
            $rejectedDifference[$key] = $requiredDifference[$key]
        }
        $rejectedDifference.id =
            "rejected-$($rejectedDeviation.ToLowerInvariant())"
        $rejectedDifference.deviation = $rejectedDeviation
        $rejectedManifest = [ordered]@{
            cases = @($selfTestManifest.cases)
            expectedDifferences = @($rejectedDifference)
        }
        $rejected = $false
        try {
            Test-DifferentialManifestDefinition `
                -Manifest $rejectedManifest `
                -DeviationRegistry $DeviationRegistry
        } catch {
            $rejected = if ($rejectedDeviation -ceq "NOTE-002") {
                $_.Exception.Message -match "non-approved status"
            } else {
                $_.Exception.Message -match "unknown deviation"
            }
        }
        if (-not $rejected) {
            throw (
                "Deviation-registry self-test accepted $rejectedDeviation"
            )
        }
    }

    return [ordered]@{
        passed = $true
        invalidUtf8CollisionDetected = $true
        matchingTimeoutsRejected = $true
        emptyManifestRejected = $true
        unconsumedDeviationRejected = $true
        optionalUnusedDeviationAccepted = $true
        nonApprovedDeviationRejected = $true
        unknownDeviationRejected = $true
        mixedEolVectorBase64 = "QQ0KQg1DCv8="
        normalizedVectorBase64 = "QQpCCkMK/w=="
        normalizedVectorSha256 =
            "49809435d2a0732d206decac3ab32cd18b93ab59b4d0146e22ba5b1b0126ad2e"
    }
}

function Test-ExpectedCaseDifference {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Expected,
        [Parameter(Mandatory = $true)]
        [object[]]$Mismatches,
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Reference,
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Candidate
    )

    $actualFields = @($Mismatches | ForEach-Object field)
    if (($actualFields -join ",") -cne (@($Expected.fields) -join ",")) {
        return $false
    }
    foreach ($field in $actualFields) {
        switch ($field) {
            "stdout" {
                if ($Expected.referenceBase64 -cne $Reference.stdout.base64 -or
                    $Expected.candidateBase64 -cne $Candidate.stdout.base64 -or
                    $Expected.referenceSha256 -cne $Reference.stdout.sha256 -or
                    $Expected.candidateSha256 -cne $Candidate.stdout.sha256) {
                    return $false
                }
            }
            "stderr" {
                if ($Expected.referenceBase64 -cne $Reference.stderr.base64 -or
                    $Expected.candidateBase64 -cne $Candidate.stderr.base64 -or
                    $Expected.referenceSha256 -cne $Reference.stderr.sha256 -or
                    $Expected.candidateSha256 -cne $Candidate.stderr.sha256) {
                    return $false
                }
            }
            "exitStatus" {
                if ($Expected.referenceExitStatus -ne $Reference.exitStatus -or
                    $Expected.candidateExitStatus -ne $Candidate.exitStatus) {
                    return $false
                }
            }
            "outcome" {
                if ($Expected.referenceOutcome -cne $Reference.outcome -or
                    $Expected.candidateOutcome -cne $Candidate.outcome) {
                    return $false
                }
            }
            default {
                return $false
            }
        }
    }
    return $true
}

$deviationLogFile =
    Resolve-RootedPath -Path $DeviationLogPath -MustExist
$deviationRegistry = Read-DeviationRegistry -Path $deviationLogFile
$comparatorSelfTest = Test-ByteComparatorInfrastructure `
    -DeviationRegistry $deviationRegistry
if ($ComparatorSelfTestOnly) {
    $comparatorSelfTest | ConvertTo-Json -Depth 4
    exit 0
}

$runningOnWindows = $env:OS -eq "Windows_NT"
$defaultExecutable = if ($runningOnWindows) {
    "target/x86_64-pc-windows-msvc/debug/lua_app.exe"
} else {
    "target/debug/lua_app"
}
if ([string]::IsNullOrWhiteSpace($CandidateLua)) {
    $CandidateLua = $defaultExecutable
}
if ([string]::IsNullOrWhiteSpace($OfficialLua) -and
    -not [string]::IsNullOrWhiteSpace($env:LUA51_REFERENCE)) {
    $OfficialLua = $env:LUA51_REFERENCE
}
if ([string]::IsNullOrWhiteSpace($CppRoot)) {
    if (-not [string]::IsNullOrWhiteSpace($env:LUA_CPP_ORACLE_ROOT)) {
        $CppRoot = $env:LUA_CPP_ORACLE_ROOT
    } else {
        $CppRoot = Join-Path (Split-Path -Parent $Root) "lua_cpp"
    }
}
if ([string]::IsNullOrWhiteSpace($CppLua)) {
    if (-not [string]::IsNullOrWhiteSpace($env:LUA_CPP_ORACLE_BIN)) {
        $CppLua = $env:LUA_CPP_ORACLE_BIN
    } else {
        $cppExecutableRelative = if ($runningOnWindows) {
            "bin/lua_app.exe"
        } else {
            "bin/lua_app"
        }
        $CppLua = Join-Path $CppRoot $cppExecutableRelative
    }
}

$casesFile = Resolve-RootedPath -Path $CasesPath -MustExist
$versionProbeFile = Resolve-RootedPath -Path $VersionProbePath -MustExist
$manifest = Get-Content -LiteralPath $casesFile -Raw | ConvertFrom-Json
if ($manifest.schemaVersion -ne 1) {
    throw "Unsupported differential case schemaVersion: $($manifest.schemaVersion)"
}
if (@($Lane).Count -eq 0) {
    throw "At least one differential lane must be requested"
}
$requestedLaneIds = [System.Collections.Generic.HashSet[string]]::new(
    [System.StringComparer]::Ordinal
)
foreach ($laneId in @($Lane)) {
    if ($laneId -cnotin @("official-lua51", "cpp-87c15e6")) {
        throw "Differential lane must use its canonical id casing: $laneId"
    }
    if (-not $requestedLaneIds.Add([string]$laneId)) {
        throw "Duplicate differential lane requested: $laneId"
    }
}
Test-DifferentialManifestDefinition `
    -Manifest $manifest `
    -DeviationRegistry $deviationRegistry

$expectedConsumption =
    [System.Collections.Generic.Dictionary[string, int]]::new(
        [System.StringComparer]::Ordinal
    )
foreach ($expected in @($manifest.expectedDifferences)) {
    $expectedConsumption.Add([string]$expected.id, 0)
}
$stdoutNormalization = @($manifest.normalization.stdout)
$stderrNormalization = @($manifest.normalization.stderr)
foreach ($rule in @($stdoutNormalization + $stderrNormalization)) {
    if ($rule -ne "crlf-to-lf") {
        throw "Unsupported differential normalization rule: $rule"
    }
}
$normalizeStdout = $stdoutNormalization -contains "crlf-to-lf"
$normalizeStderr = $stderrNormalization -contains "crlf-to-lf"

$infrastructureFailures = [System.Collections.Generic.List[string]]::new()
$candidatePath = ""
try {
    $candidatePath = Resolve-Executable -Value $CandidateLua -Role "Rust candidate"
} catch {
    $infrastructureFailures.Add($_.Exception.Message)
}

$referencePaths = @{}
if ($Lane -contains "official-lua51") {
    try {
        $referencePaths["official-lua51"] =
            Resolve-Executable -Value $OfficialLua -Role "official Lua 5.1.5 reference"
    } catch {
        $infrastructureFailures.Add($_.Exception.Message)
    }
}
if ($Lane -contains "cpp-87c15e6") {
    $resolvedCppRoot = Resolve-RootedPath -Path $CppRoot
    if (-not (Test-Path -LiteralPath (Join-Path $resolvedCppRoot ".git"))) {
        $infrastructureFailures.Add("Missing C++ oracle checkout: $resolvedCppRoot")
    } else {
        $actualCommit = (& git -C $resolvedCppRoot rev-parse HEAD 2>$null | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) {
            $infrastructureFailures.Add("Could not read C++ oracle commit: $resolvedCppRoot")
        } elseif ($actualCommit -ne "87c15e69ceb94eb74e28226ccbefb7e196635711") {
            $infrastructureFailures.Add(
                "C++ oracle commit mismatch: expected " +
                "87c15e69ceb94eb74e28226ccbefb7e196635711, got $actualCommit"
            )
        }
    }
    try {
        $referencePaths["cpp-87c15e6"] =
            Resolve-Executable -Value $CppLua -Role "C++ 87c15e6 reference"
    } catch {
        $infrastructureFailures.Add($_.Exception.Message)
    }
}

$laneResults = [System.Collections.Generic.List[object]]::new()
if ($infrastructureFailures.Count -eq 0) {
    $candidateVersion = Invoke-ObservedProcess `
        -Executable $candidatePath `
        -Arguments @($versionProbeFile) `
        -Timeout $TimeoutSeconds
    $candidateVersionFailure = Get-ObservationInfrastructureFailure `
        -Context "candidate/_VERSION" `
        -Observation $candidateVersion
    if (-not [string]::IsNullOrWhiteSpace($candidateVersionFailure)) {
        $infrastructureFailures.Add($candidateVersionFailure)
    }

    foreach ($laneId in $Lane) {
        $referencePath = $referencePaths[$laneId]
        $referenceVersion = Invoke-ObservedProcess `
            -Executable $referencePath `
            -Arguments @($versionProbeFile) `
            -Timeout $TimeoutSeconds
        $referenceVersionFailure = Get-ObservationInfrastructureFailure `
            -Context "$laneId/reference/_VERSION" `
            -Observation $referenceVersion
        if (-not [string]::IsNullOrWhiteSpace($referenceVersionFailure)) {
            $infrastructureFailures.Add($referenceVersionFailure)
        }
        $versionCompleted =
            $referenceVersion.outcome -ceq "completed" -and
            $candidateVersion.outcome -ceq "completed"
        $versionMismatches = @(Compare-Observations `
            -Reference $referenceVersion `
            -Candidate $candidateVersion `
            -NormalizeStdout $normalizeStdout `
            -NormalizeStderr $normalizeStderr)
        $versionRawPassed = $versionMismatches.Count -eq 0
        $acceptedVersionDeviation = $null
        if ($versionCompleted -and -not $versionRawPassed) {
            foreach ($expected in @($manifest.expectedDifferences)) {
                if (-not (
                    Test-ObjectProperty -Object $expected -Name "probe"
                )) {
                    continue
                }
                $actualFields = @($versionMismatches | ForEach-Object field)
                $expectedFields = @($expected.fields)
                $referenceVersionText = Get-AsciiEvidenceText `
                    -Evidence $referenceVersion.stdout `
                    -NormalizeLineEndings $normalizeStdout
                $candidateVersionText = Get-AsciiEvidenceText `
                    -Evidence $candidateVersion.stdout `
                    -NormalizeLineEndings $normalizeStdout
                if ($expected.lane -ceq $laneId -and
                    $expected.probe -ceq "_VERSION" -and
                    ($actualFields -join ",") -ceq ($expectedFields -join ",") -and
                    $referenceVersionText -ceq $expected.referenceUtf8 -and
                    $candidateVersionText -ceq $expected.candidateUtf8) {
                    $acceptedVersionDeviation = [ordered]@{
                        id = $expected.id
                        deviation = $expected.deviation
                    }
                    $expectedConsumption[[string]$expected.id]++
                    break
                }
            }
        }
        $versionPassed =
            $versionCompleted -and (
                $versionRawPassed -or $null -ne $acceptedVersionDeviation
            )

        $caseResults = [System.Collections.Generic.List[object]]::new()
        foreach ($case in @($manifest.cases)) {
            $caseStdoutNormalization = @($stdoutNormalization)
            $caseStderrNormalization = @($stderrNormalization)
            if ($case.PSObject.Properties.Name -contains "normalization") {
                if ($case.normalization.PSObject.Properties.Name -contains "stdout") {
                    $caseStdoutNormalization = @($case.normalization.stdout)
                }
                if ($case.normalization.PSObject.Properties.Name -contains "stderr") {
                    $caseStderrNormalization = @($case.normalization.stderr)
                }
            }
            foreach ($rule in @(
                $caseStdoutNormalization + $caseStderrNormalization
            )) {
                if ($rule -ne "crlf-to-lf") {
                    throw (
                        "Unsupported differential normalization rule in case " +
                        "$($case.id): $rule"
                    )
                }
            }
            $caseNormalizeStdout =
                $caseStdoutNormalization -contains "crlf-to-lf"
            $caseNormalizeStderr =
                $caseStderrNormalization -contains "crlf-to-lf"

            $scriptPath = Resolve-RootedPath -Path $case.script -MustExist
            $reference = Invoke-ObservedProcess `
                -Executable $referencePath `
                -Arguments @($scriptPath) `
                -Timeout $TimeoutSeconds
            $candidate = Invoke-ObservedProcess `
                -Executable $candidatePath `
                -Arguments @($scriptPath) `
                -Timeout $TimeoutSeconds
            $referenceFailure = Get-ObservationInfrastructureFailure `
                -Context "$laneId/$($case.id)/reference" `
                -Observation $reference
            if (-not [string]::IsNullOrWhiteSpace($referenceFailure)) {
                $infrastructureFailures.Add($referenceFailure)
            }
            $candidateFailure = Get-ObservationInfrastructureFailure `
                -Context "$laneId/$($case.id)/candidate" `
                -Observation $candidate
            if (-not [string]::IsNullOrWhiteSpace($candidateFailure)) {
                $infrastructureFailures.Add($candidateFailure)
            }
            $caseCompleted =
                $reference.outcome -ceq "completed" -and
                $candidate.outcome -ceq "completed"
            $mismatches = @(
                Compare-Observations `
                    -Reference $reference `
                    -Candidate $candidate `
                    -NormalizeStdout $caseNormalizeStdout `
                    -NormalizeStderr $caseNormalizeStderr
            )
            $rawCasePassed = $mismatches.Count -eq 0
            $acceptedCaseDeviation = $null
            if ($caseCompleted -and -not $rawCasePassed) {
                foreach ($expected in @($manifest.expectedDifferences)) {
                    if (-not (
                        Test-ObjectProperty -Object $expected -Name "case"
                    )) {
                        continue
                    }
                    if ($expected.lane -ceq $laneId -and
                        $expected.case -ceq $case.id -and
                        (Test-ExpectedCaseDifference `
                            -Expected $expected `
                            -Mismatches $mismatches `
                            -Reference $reference `
                            -Candidate $candidate)) {
                        $acceptedCaseDeviation = [ordered]@{
                            id = $expected.id
                            deviation = $expected.deviation
                            reason = $expected.reason
                        }
                        $expectedConsumption[[string]$expected.id]++
                        break
                    }
                }
            }
            $casePassed =
                $caseCompleted -and (
                    $rawCasePassed -or $null -ne $acceptedCaseDeviation
                )

            $caseResults.Add([ordered]@{
                id = $case.id
                script = $case.script
                evidence = @($case.evidence)
                normalization = [ordered]@{
                    stdout = @($caseStdoutNormalization)
                    stderr = @($caseStderrNormalization)
                }
                passed = $casePassed
                rawPassed = $rawCasePassed
                acceptedDeviation = $acceptedCaseDeviation
                mismatches = @($mismatches)
                reference = $reference
                candidate = $candidate
                semanticEvidence = [ordered]@{
                    reference = Get-SemanticEvidence `
                        -CaseId $case.id `
                        -Observation $reference
                    candidate = Get-SemanticEvidence `
                        -CaseId $case.id `
                        -Observation $candidate
                }
            })
        }

        $failedCases = @($caseResults | Where-Object { -not $_.passed })
        $laneResults.Add([ordered]@{
            id = $laneId
            referenceExecutable = $referencePath
            candidateExecutable = $candidatePath
            passed = $versionPassed -and $failedCases.Count -eq 0
            versionProbe = [ordered]@{
                passed = $versionPassed
                rawPassed = $versionRawPassed
                acceptedDeviation = $acceptedVersionDeviation
                mismatches = @($versionMismatches)
                reference = $referenceVersion
                candidate = $candidateVersion
            }
            cases = @($caseResults)
        })
    }
}

$consumptionFailures = @(
    Get-ExpectedDifferenceConsumptionFailures `
        -ExpectedDifferences @($manifest.expectedDifferences) `
        -Consumption $expectedConsumption `
        -RequestedLanes @($Lane)
)
foreach ($failure in $consumptionFailures) {
    $infrastructureFailures.Add($failure)
}

$semanticFailures = @($laneResults | Where-Object { -not $_.passed })
$document = [ordered]@{
    schemaVersion = 1
    channel = "lua51-differential"
    generatedAtUtc = [DateTime]::UtcNow.ToString("o")
    passed = $infrastructureFailures.Count -eq 0 -and $semanticFailures.Count -eq 0
    timeoutSeconds = $TimeoutSeconds
    manifest = $CasesPath
    deviationRegistry = [ordered]@{
        path = $DeviationLogPath
        sha256 = (
            Get-FileHash -LiteralPath $deviationLogFile -Algorithm SHA256
        ).Hash.ToLowerInvariant()
        approvedIds = @($deviationRegistry.Keys | Sort-Object | Where-Object {
            $deviationRegistry[$_] -ceq "approved"
        })
    }
    normalization = [ordered]@{
        stdout = @($stdoutNormalization)
        stderr = @($stderrNormalization)
    }
    comparatorSelfTest = $comparatorSelfTest
    lanesRequested = @($Lane)
    infrastructureFailures = @($infrastructureFailures)
    expectedDifferences = @($manifest.expectedDifferences | ForEach-Object {
        $currentExpected = $_
        [ordered]@{
            id = $currentExpected.id
            lane = $currentExpected.lane
            deviation = $currentExpected.deviation
            optional = Get-ExpectedDifferenceOptional -Expected $currentExpected
            applicable = @($Lane | Where-Object {
                $_ -ceq [string]$currentExpected.lane
            }).Count -gt 0
            consumed = $expectedConsumption[[string]$currentExpected.id]
        }
    })
    lanes = @($laneResults)
}

$resolvedResultPath = Resolve-RootedPath -Path $ResultPath
$resultParent = Split-Path -Parent $resolvedResultPath
if (-not (Test-Path -LiteralPath $resultParent)) {
    New-Item -ItemType Directory -Path $resultParent -Force | Out-Null
}
$json = $document | ConvertTo-Json -Depth 16
[System.IO.File]::WriteAllText(
    $resolvedResultPath,
    $json + [Environment]::NewLine,
    [System.Text.UTF8Encoding]::new($false)
)

Write-Host "[INFO] Differential report: $resolvedResultPath"
foreach ($laneResult in $laneResults) {
    $passedCases = @($laneResult.cases | Where-Object passed).Count
    $versionStatus = if ($laneResult.versionProbe.rawPassed) {
        "match"
    } elseif ($null -ne $laneResult.versionProbe.acceptedDeviation) {
        "accepted:$($laneResult.versionProbe.acceptedDeviation.deviation)"
    } else {
        "different"
    }
    Write-Host (
        "[INFO] {0}: cases {1}/{2}, _VERSION={3}" -f
        $laneResult.id,
        $passedCases,
        $laneResult.cases.Count,
        $versionStatus
    )
}

if ($infrastructureFailures.Count -gt 0) {
    Write-Host "[ERROR] Differential infrastructure is not runnable:"
    $infrastructureFailures | ForEach-Object { Write-Host " - $_" }
    exit 2
}
if ($semanticFailures.Count -gt 0) {
    Write-Host "[FAIL] Lua 5.1 differential mismatches remain"
    exit 1
}

Write-Host "[OK] All requested Lua 5.1 differential lanes passed"
