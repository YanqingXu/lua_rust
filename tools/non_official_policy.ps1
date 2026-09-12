# Shared by the M0 artifact check and its behavioral self-test. This module never
# updates the baseline: relaxing debt requires an explicit, reviewed file edit.
function Test-NonOfficialArtifact {
    param(
        [Parameter(Mandatory = $true)]$Artifact,
        [Parameter(Mandatory = $true)]$Manifest,
        [Parameter(Mandatory = $true)]$Policy
    )

    $failures = [System.Collections.Generic.List[string]]::new()
    $accepted = [System.Collections.Generic.List[object]]::new()
    $resolved = [System.Collections.Generic.List[string]]::new()
    $seen = @{}
    $approved = @{}
    $expected = @{}
    $counts = @{ selected = 0; executed = 0; helpers_skipped = 0; matches = 0;
        differences = 0; rust_only_expected = 0; runner_errors = 0; timed_out = 0 }

    if ($Policy.schema_version -ne 1 -or $Policy.suite -ne "non-official") {
        throw "unsupported non-official debt policy schema or suite"
    }
    if ($Artifact.schema_version -ne "lua-fixture-differential/v1" -or
        $Artifact.suite -ne "non-official") {
        throw "unsupported non-official artifact schema or suite"
    }
    if ($Artifact.fixture_inventory.current_total -ne $Manifest.inventory.current_total) {
        $failures.Add("artifact fixture inventory does not match the manifest")
    }
    foreach ($fixture in $Manifest.cases) {
        $path = $fixture.path.Replace('\', '/')
        if ($path.StartsWith("tests/lua/official/") -or
            $path.StartsWith("tests/lua/differential/")) { continue }
        if ($expected.ContainsKey($fixture.id)) { throw "duplicate manifest case: $($fixture.id)" }
        $expected[$fixture.id] = $fixture
    }
    foreach ($requiredCase in $Policy.required_cases) {
        $id = $requiredCase.id
        if (-not $expected.ContainsKey($id)) {
            $failures.Add("required baseline case missing from manifest: $id")
            continue
        }
        foreach ($field in @("kind", "expected_exit", "compare_stdout", "compare_stderr", "compare_side_effects")) {
            if ($expected[$id].$field -cne $requiredCase.$field) {
                $failures.Add("baseline case execution contract changed: $id/$field")
            }
        }
    }
    foreach ($debt in $Policy.debts) {
        if ($approved.ContainsKey($debt.id)) { throw "duplicate debt case: $($debt.id)" }
        if (-not $expected.ContainsKey($debt.id)) { throw "unknown debt case: $($debt.id)" }
        if (@($debt.channels).Count -ne 1 -or $debt.channels[0] -ne "stdout") {
            throw "only bounded stdout debt is supported: $($debt.id)"
        }
        if ([string]::IsNullOrWhiteSpace($debt.reason)) { throw "missing debt reason: $($debt.id)" }
        foreach ($engine in @("lua_rust", "lua_cpp")) {
            $pattern = $debt.stdout_patterns.$engine
            if (-not $pattern.StartsWith('\A') -or -not $pattern.EndsWith('\z')) {
                throw "debt signature must cover complete stdout: $($debt.id)/$engine"
            }
            $null = [regex]::new($pattern)
        }
        $approved[$debt.id] = $debt
    }

    foreach ($case in $Artifact.cases) {
        $counts.selected++
        $id = [string]$case.id
        if ($seen.ContainsKey($id)) { $failures.Add("duplicate artifact case: $id"); continue }
        $seen[$id] = $true
        if (-not $expected.ContainsKey($id)) { $failures.Add("unexpected artifact case: $id"); continue }
        $fixture = $expected[$id]
        if ($case.path -cne $fixture.path -or $case.kind -cne $fixture.kind -or
            $case.expected_exit -ne $fixture.expected_exit) {
            $failures.Add("case metadata does not match the manifest: $id")
        }
        if ($case.kind -eq "helper") {
            $counts.helpers_skipped++
            if ($case.status -ne "helper-skipped" -or $null -ne $case.lua_rust -or
                $null -ne $case.lua_cpp -or $null -ne $case.comparison) {
                $failures.Add("invalid helper record: $id")
            }
            continue
        }
        $counts.executed++
        switch ($case.status) {
            "match" { $counts.matches++ }
            "difference" { $counts.differences++ }
            "runner-error" { $counts.runner_errors++ }
            "rust-only-expected" { $counts.rust_only_expected++ }
            default { $failures.Add("unexpected execution status for ${id}: $($case.status)") }
        }

        $validExecution = $true
        $caseTimedOut = $false
        foreach ($engine in @("lua_rust", "lua_cpp")) {
            $execution = $case.$engine
            if ($null -eq $execution) {
                $failures.Add("missing engine execution: $id/$engine")
                $validExecution = $false
                continue
            }
            if ($execution.timed_out) { $caseTimedOut = $true }
            # 126+ are shell/signal/panic exits; negative/large codes also cover
            # Windows exceptions. Never allow these via semantic-debt entries.
            if ($execution.timed_out -or $null -ne $execution.spawn_error -or
                $null -eq $execution.exit_code -or $execution.exit_code -lt 0 -or
                $execution.exit_code -ge 126 -or $execution.exit_code -eq 101 -or
                $execution.exit_code -ne $fixture.expected_exit -or
                $execution.expected_exit_matched -ne $true) {
                $failures.Add("abnormal or unexpected process exit: $id/$engine (exit=$($execution.exit_code))")
                $validExecution = $false
            }
            if ($execution.stderr.lossy_text -match
                "(?m)^thread '.+' panicked at|panic in a function that cannot unwind|fatal runtime error:|ERROR: AddressSanitizer|runtime error:.*undefined behavior") {
                $failures.Add("host panic or sanitizer failure: $id/$engine")
                $validExecution = $false
            }
        }
        if ($caseTimedOut) { $counts.timed_out++ }
        if (-not $validExecution) { continue }
        $comparison = $case.comparison
        if ($null -eq $comparison) { $failures.Add("missing comparison: $id"); continue }
        $channels = [System.Collections.Generic.List[string]]::new()
        if ($comparison.timeout_equal -ne $true -or $comparison.exit_code_equal -ne $true) {
            $failures.Add("process comparison failed: $id")
        }
        foreach ($channel in @("stdout", "stderr", "side_effects")) {
            $enabled = $fixture.("compare_$channel")
            if ($comparison.("${channel}_compared") -ne $enabled) {
                $failures.Add("comparison channel disabled or changed: $id/$channel")
            }
            if ($enabled -and $comparison.("${channel}_equal") -ne $true) {
                $channels.Add($channel)
            }
        }
        if ($channels.Count -eq 0) {
            if ($case.status -ne "match" -or $comparison.matched -ne $true) {
                $failures.Add("inconsistent matching case: $id")
            } elseif ($approved.ContainsKey($id)) {
                # Delete this entry when landing its fix, so a future recurrence
                # cannot be accepted as historical debt again.
                $resolved.Add($id)
                $failures.Add("resolved debt must be removed from policy: $id")
            }
            continue
        }
        if ($case.status -ne "difference" -or $comparison.matched -ne $false) {
            $failures.Add("inconsistent differing case: $id")
        }
        if (-not $approved.ContainsKey($id)) {
            $failures.Add("unapproved semantic regression: $id ($($channels -join ', '))")
            continue
        }
        $debt = $approved[$id]
        if ($channels.Count -ne 1 -or $channels[0] -ne "stdout") {
            $failures.Add("new difference channel for ${id}: $($channels -join ', ')")
            continue
        }
        $signatureMatches = $true
        foreach ($engine in @("lua_rust", "lua_cpp")) {
            $stdout = $case.$engine.stdout.normalized_lossy_text.Replace("`r`n", "`n")
            if (-not [regex]::IsMatch($stdout, $debt.stdout_patterns.$engine,
                    [System.Text.RegularExpressions.RegexOptions]::CultureInvariant,
                    [TimeSpan]::FromSeconds(1))) {
                $failures.Add("changed debt stdout signature: $id/$engine")
                $signatureMatches = $false
            }
        }
        if ($signatureMatches) {
            $accepted.Add([ordered]@{ id = $id; channels = @("stdout"); reason = $debt.reason })
        }
    }
    foreach ($id in $expected.Keys) {
        if (-not $seen.ContainsKey($id)) { $failures.Add("manifest case missing from artifact: $id") }
    }
    foreach ($key in $counts.Keys) {
        if ($Artifact.summary.$key -ne $counts[$key]) {
            $failures.Add("summary accounting mismatch: $key")
        }
    }
    if ($counts.runner_errors -gt 0 -or $counts.rust_only_expected -gt 0) {
        $failures.Add("runner errors or incomplete dual-oracle executions are not semantic debt")
    }
    return [pscustomobject][ordered]@{
        schemaVersion = 1
        channel = "non-official-policy"
        passed = $failures.Count -eq 0
        summary = $counts
        acceptedDebt = @($accepted)
        resolvedDebt = @($resolved)
        failures = @($failures)
    }
}
