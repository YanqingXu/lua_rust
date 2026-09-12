# Non-official regression policy, 2026-09-12

The A09 audit at `503bf99d12a5ebae81862ad3ea4dbf7e97af38ca` observed
81 matching executions, 11 differences, and nine skipped helper files. M0
previously accepted any number of non-official differences as migration debt.
The initial `tests/compatibility/non-official-debt.json` froze ten of those
historical stdout differences, identified by case ID, channel, and full output
signature. Its provenance includes the audit artifact SHA-256. It does not
grant permission to introduce new differences.

Seven cases differ in implementation-specific GC memory accounting; only their
existing memory measurements may vary. Two differ in userdata formatting; only
the existing Rust allocation addresses may vary. `tables.metamethod-arith`
was subsequently fixed by the shared number formatter and structured error
origin. Exact Debug/Release/C++ stdout, stderr and exit verification in
`target/error-provenance-evidence/verification.json` justified removing its
exemption completely. Nine historical debts remain; all other output remains
fixed. Line endings and checkout-root paths are portable.
Exit codes, stderr, and filesystem effects have no debt exemptions.

The eleventh old difference, `integration.env-metatable-loader`, exited early
in both engines because its restricted environment lacked `tostring`. This is
an invalid fixture execution, not approved semantic debt. Its fixture receives
the missing dependency and keeps `expected_exit = 0`; the original audit
artifact is correctly rejected by the new checker.

All original cases retain their ID, execution kind, expected exit, and comparison
switches. Missing records and replacement or skipped cases fail. Newly added
cases must pass. A resolved debt must be removed from the policy so it cannot
later become an accepted regression. Changes to the fixture manifest and this
debt file now follow the existing oracle baseline-change review policy.

`tools/test_non_official_policy.ps1` checks same-count case swaps, added failing
fixtures, changed signatures and channels, unexpected and abnormal exits,
panic diagnostics, missing/duplicate records, summary corruption, and resolved
debt removal. M0 executes this self-test and writes a separate policy artifact;
`--allow-differences` is used only to collect runner evidence before policy
validation. Rust quality checks also run doctests outside the nextest branch,
and the Miri CI job executes `runtime_access_contract` through the public API.

The independent official source lane classifies all 24 pinned source files as
19 executable entries, two helpers and three explicit exclusions. Eight
previously verified source entries become strict CI checks. Its classification
manifest is included in baseline-change protection. Explicitly required cases
cannot be changed to excluded/helper roles and silently disappear: the runner
rejects this before execution, and the fifth runner self-test locks the rule.
Neither the C++ pin nor upstream Lua source bytes have changed.
