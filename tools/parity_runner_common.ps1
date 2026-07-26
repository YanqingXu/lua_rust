Set-StrictMode -Version Latest

function Resolve-ParityPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$BasePath
    )

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }

    return [System.IO.Path]::GetFullPath((Join-Path $BasePath $Path))
}

function New-ParityDirectory {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        New-Item -ItemType Directory -Path $Path -Force | Out-Null
    }
}

function Write-ParityText {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [AllowEmptyString()]
        [string]$Text = ""
    )

    $parent = Split-Path -Parent $Path
    if ($parent) {
        New-ParityDirectory -Path $parent
    }

    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Text, $encoding)
}

function Write-ParityJson {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Value
    )

    $json = $Value | ConvertTo-Json -Depth 64
    Write-ParityText -Path $Path -Text $json
}

function ConvertTo-NativeArgument {
    param(
        [AllowEmptyString()]
        [string]$Argument
    )

    if ($Argument -notmatch '[\s"]') {
        return $Argument
    }

    $builder = New-Object System.Text.StringBuilder
    [void]$builder.Append('"')
    $backslashes = 0

    foreach ($character in $Argument.ToCharArray()) {
        if ($character -eq '\') {
            $backslashes++
            continue
        }

        if ($character -eq '"') {
            [void]$builder.Append((('\' * (($backslashes * 2) + 1)) -join ''))
            [void]$builder.Append('"')
            $backslashes = 0
            continue
        }

        if ($backslashes -gt 0) {
            [void]$builder.Append((('\' * $backslashes) -join ''))
            $backslashes = 0
        }
        [void]$builder.Append($character)
    }

    if ($backslashes -gt 0) {
        [void]$builder.Append((('\' * ($backslashes * 2)) -join ''))
    }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Format-ParityCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Executable,

        [string[]]$Arguments = @()
    )

    $parts = @((ConvertTo-NativeArgument -Argument $Executable))
    foreach ($argument in $Arguments) {
        $parts += ConvertTo-NativeArgument -Argument $argument
    }
    return $parts -join " "
}

function Invoke-ParityProcess {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Executable,

        [string[]]$Arguments = @(),

        [Parameter(Mandatory = $true)]
        [string]$WorkingDirectory,

        [ValidateRange(1, 3600)]
        [int]$TimeoutSeconds = 30
    )

    $command = [ordered]@{
        executable = $Executable
        arguments  = @($Arguments)
        display    = Format-ParityCommand -Executable $Executable -Arguments $Arguments
    }
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $process = $null
    $stdout = ""
    $stderr = ""
    $exitCode = $null
    $timedOut = $false
    $startError = $null

    try {
        $startInfo = New-Object System.Diagnostics.ProcessStartInfo
        $startInfo.FileName = $Executable
        $startInfo.WorkingDirectory = $WorkingDirectory
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true

        if ($startInfo.PSObject.Properties.Name -contains "ArgumentList") {
            foreach ($argument in $Arguments) {
                [void]$startInfo.ArgumentList.Add($argument)
            }
        }
        else {
            $startInfo.Arguments = (($Arguments | ForEach-Object {
                ConvertTo-NativeArgument -Argument $_
            }) -join " ")
        }

        $process = New-Object System.Diagnostics.Process
        $process.StartInfo = $startInfo
        if (-not $process.Start()) {
            throw "process start returned false"
        }

        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $timedOut = $true
            try {
                $process.Kill()
            }
            catch {
                # The process may have exited between WaitForExit and Kill.
            }
        }

        $process.WaitForExit()
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if (-not $timedOut) {
            $exitCode = $process.ExitCode
        }
    }
    catch {
        $startError = $_.Exception.Message
    }
    finally {
        $stopwatch.Stop()
        if ($null -ne $process) {
            $process.Dispose()
        }
    }

    return [pscustomobject][ordered]@{
        command    = $command
        stdout     = $stdout
        stderr     = $stderr
        exitCode   = $exitCode
        timedOut   = $timedOut
        durationMs = [int64]$stopwatch.ElapsedMilliseconds
        startError = $startError
    }
}

function Get-ParityRelativePath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root,

        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $pathFull = [System.IO.Path]::GetFullPath($Path)
    $prefix = $rootFull + [System.IO.Path]::DirectorySeparatorChar
    if ($pathFull.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        return $pathFull.Substring($prefix.Length)
    }
    return [System.IO.Path]::GetFileName($pathFull)
}

function Get-ParityCaseId {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RelativePath
    )

    $stem = [System.IO.Path]::ChangeExtension($RelativePath, $null)
    $safe = ($stem -replace '[^A-Za-z0-9._-]+', '_').Trim('_')
    if (-not $safe) {
        $safe = "case"
    }

    $encoding = [System.Text.Encoding]::UTF8
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = $sha.ComputeHash($encoding.GetBytes($RelativePath.Replace('\', '/')))
        $hash = ([System.BitConverter]::ToString($bytes)).Replace("-", "").Substring(0, 8).ToLowerInvariant()
    }
    finally {
        $sha.Dispose()
    }

    return "$safe-$hash"
}

function Get-ParityFileSha256 {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Select-ParityCorpus {
    param(
        [Parameter(Mandatory = $true)]
        [string]$InputPath,

        [ValidateSet("Representative", "Full")]
        [string]$Mode = "Representative",

        [ValidateRange(1, 100000)]
        [int]$RepresentativeCount = 12,

        [string]$RepresentativeManifest = ""
    )

    $inputItem = Get-Item -LiteralPath $InputPath -ErrorAction Stop
    if (-not $inputItem.PSIsContainer) {
        if ($inputItem.Extension -ne ".lua") {
            throw "Input file is not a .lua source: $InputPath"
        }
        return @($inputItem)
    }

    $allFiles = @(Get-ChildItem -LiteralPath $inputItem.FullName -File -Filter "*.lua" -Recurse |
        Sort-Object FullName)
    if ($allFiles.Count -eq 0) {
        throw "No .lua files found under: $InputPath"
    }
    if ($Mode -eq "Full") {
        return $allFiles
    }

    if ($RepresentativeManifest) {
        $manifestPath = [System.IO.Path]::GetFullPath($RepresentativeManifest)
        if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
            throw "Representative manifest not found: $manifestPath"
        }
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        $entries = if ($manifest -is [System.Array]) { @($manifest) } else { @($manifest.files) }
        if ($entries.Count -eq 0) {
            throw "Representative manifest contains no files: $manifestPath"
        }

        $selected = @()
        foreach ($entry in $entries) {
            $entryText = [string]$entry
            $candidate = if ([System.IO.Path]::IsPathRooted($entryText)) {
                [System.IO.Path]::GetFullPath($entryText)
            }
            else {
                [System.IO.Path]::GetFullPath((Join-Path $inputItem.FullName $entryText))
            }
            if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
                throw "Representative corpus entry not found: $entryText"
            }
            if ([System.IO.Path]::GetExtension($candidate) -ne ".lua") {
                throw "Representative corpus entry is not Lua: $entryText"
            }
            $selected += Get-Item -LiteralPath $candidate
        }
        return @($selected | Sort-Object FullName -Unique)
    }

    $patterns = @(
        'opcode|bytecode',
        'closure|upvalue',
        'vararg|call',
        'table|meta',
        'loop|control|branch',
        'error|pcall',
        'coroutine|yield|resume',
        'string|pattern',
        'number|arith|math',
        'return|multret'
    )
    $picked = New-Object System.Collections.ArrayList
    $pickedPaths = @{}
    foreach ($pattern in $patterns) {
        if ($picked.Count -ge $RepresentativeCount) {
            break
        }
        $match = $allFiles | Where-Object {
            $_.FullName -match $pattern -and -not $pickedPaths.ContainsKey($_.FullName)
        } | Select-Object -First 1
        if ($null -ne $match) {
            [void]$picked.Add($match)
            $pickedPaths[$match.FullName] = $true
        }
    }
    foreach ($file in $allFiles) {
        if ($picked.Count -ge $RepresentativeCount) {
            break
        }
        if (-not $pickedPaths.ContainsKey($file.FullName)) {
            [void]$picked.Add($file)
            $pickedPaths[$file.FullName] = $true
        }
    }

    return @($picked | Sort-Object FullName)
}

function ConvertTo-ParityMap {
    param(
        [AllowNull()]
        [object]$Value
    )

    if ($null -eq $Value) {
        return $null
    }
    if ($Value -is [System.Collections.IDictionary]) {
        $map = [ordered]@{}
        foreach ($key in @($Value.Keys | Sort-Object)) {
            $map[[string]$key] = ConvertTo-ParityMap -Value $Value[$key]
        }
        return $map
    }
    if ($Value.GetType() -eq [System.Management.Automation.PSCustomObject]) {
        $map = [ordered]@{}
        foreach ($property in @($Value.PSObject.Properties | Sort-Object Name)) {
            $map[$property.Name] = ConvertTo-ParityMap -Value $property.Value
        }
        return $map
    }
    if ($Value -is [System.Collections.IEnumerable] -and $Value -isnot [string]) {
        $items = @($Value | ForEach-Object { ConvertTo-ParityMap -Value $_ })
        return ,$items
    }
    return $Value
}

function Compare-ParityValue {
    param(
        [AllowNull()]
        [object]$Left,

        [AllowNull()]
        [object]$Right,

        [string]$Path = '$',

        [ValidateRange(1, 100000)]
        [int]$MaximumDifferences = 500
    )

    $differences = New-Object System.Collections.ArrayList

    function Add-ComparisonDifference {
        param(
            [string]$DifferencePath,
            [string]$Kind,
            [AllowNull()][object]$LeftValue,
            [AllowNull()][object]$RightValue,
            [string]$Message
        )

        if ($differences.Count -lt $MaximumDifferences) {
            [void]$differences.Add([pscustomobject][ordered]@{
                path    = $DifferencePath
                kind    = $Kind
                left    = $LeftValue
                right   = $RightValue
                message = $Message
            })
        }
    }

    function Compare-Node {
        param(
            [AllowNull()][object]$LeftNode,
            [AllowNull()][object]$RightNode,
            [string]$NodePath
        )

        if ($differences.Count -ge $MaximumDifferences) {
            return
        }
        if ($null -eq $LeftNode -and $null -eq $RightNode) {
            return
        }
        if ($null -eq $LeftNode -or $null -eq $RightNode) {
            Add-ComparisonDifference -DifferencePath $NodePath -Kind "value" `
                -LeftValue $LeftNode -RightValue $RightNode -Message "one side is null"
            return
        }

        $leftMap = ConvertTo-ParityMap -Value $LeftNode
        $rightMap = ConvertTo-ParityMap -Value $RightNode
        $leftIsMap = $leftMap -is [System.Collections.IDictionary]
        $rightIsMap = $rightMap -is [System.Collections.IDictionary]
        if ($leftIsMap -or $rightIsMap) {
            if (-not ($leftIsMap -and $rightIsMap)) {
                Add-ComparisonDifference -DifferencePath $NodePath -Kind "type" `
                    -LeftValue $leftMap -RightValue $rightMap -Message "object/scalar type mismatch"
                return
            }

            $keys = @($leftMap.Keys + $rightMap.Keys | Sort-Object -Unique)
            foreach ($key in $keys) {
                $leftHas = $leftMap.Contains($key)
                $rightHas = $rightMap.Contains($key)
                $childPath = "$NodePath.$key"
                if (-not $leftHas -or -not $rightHas) {
                    Add-ComparisonDifference -DifferencePath $childPath -Kind "missing-field" `
                        -LeftValue $(if ($leftHas) { $leftMap[$key] } else { $null }) `
                        -RightValue $(if ($rightHas) { $rightMap[$key] } else { $null }) `
                        -Message "field is absent on one side"
                    continue
                }
                Compare-Node -LeftNode $leftMap[$key] -RightNode $rightMap[$key] -NodePath $childPath
            }
            return
        }

        $leftIsArray = $leftMap -is [System.Array]
        $rightIsArray = $rightMap -is [System.Array]
        if ($leftIsArray -or $rightIsArray) {
            if (-not ($leftIsArray -and $rightIsArray)) {
                Add-ComparisonDifference -DifferencePath $NodePath -Kind "type" `
                    -LeftValue $leftMap -RightValue $rightMap -Message "array/scalar type mismatch"
                return
            }
            if ($leftMap.Count -ne $rightMap.Count) {
                Add-ComparisonDifference -DifferencePath "$NodePath.length" -Kind "length" `
                    -LeftValue $leftMap.Count -RightValue $rightMap.Count -Message "array length mismatch"
            }
            $count = [System.Math]::Min($leftMap.Count, $rightMap.Count)
            for ($index = 0; $index -lt $count; $index++) {
                Compare-Node -LeftNode $leftMap[$index] -RightNode $rightMap[$index] `
                    -NodePath "$NodePath[$index]"
            }
            return
        }

        if ([string]$leftMap -cne [string]$rightMap) {
            Add-ComparisonDifference -DifferencePath $NodePath -Kind "value" `
                -LeftValue $leftMap -RightValue $rightMap -Message "scalar value mismatch"
        }
    }

    Compare-Node -LeftNode $Left -RightNode $Right -NodePath $Path
    return [pscustomobject][ordered]@{
        items     = @($differences)
        truncated = ($differences.Count -ge $MaximumDifferences)
    }
}

function Save-ParityExecution {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Execution,

        [Parameter(Mandatory = $true)]
        [string]$CaseDirectory,

        [Parameter(Mandatory = $true)]
        [string]$Side
    )

    $stdoutPath = Join-Path $CaseDirectory "$Side.stdout.txt"
    $stderrPath = Join-Path $CaseDirectory "$Side.stderr.txt"
    Write-ParityText -Path $stdoutPath -Text $Execution.stdout
    Write-ParityText -Path $stderrPath -Text $Execution.stderr

    return [pscustomobject][ordered]@{
        command    = $Execution.command
        stdout     = $Execution.stdout
        stderr     = $Execution.stderr
        stdoutFile = $stdoutPath
        stderrFile = $stderrPath
        exitCode   = $Execution.exitCode
        timedOut   = $Execution.timedOut
        durationMs = $Execution.durationMs
        startError = $Execution.startError
    }
}
