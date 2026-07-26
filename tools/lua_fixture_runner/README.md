# Lua fixture runner

This is the process-level compatibility runner for `tests/lua`. It validates
that every Lua fixture is classified by
`tests/compatibility/lua_fixtures.json`, runs executable fixtures in child
processes, and writes a machine-readable JSON report.

From the `lua_rust` repository root:

```powershell
cargo build --package lua_app
cargo run --manifest-path tools/lua_fixture_runner/Cargo.toml -- `
  --rust target/x86_64-pc-windows-msvc/debug/lua_app.exe `
  --cpp ../lua_cpp/bin/lua_app.exe `
  --artifact target/compatibility/non-official.json
```

On Unix, pass the corresponding executable paths. The runner itself does not
assume a shell and is cross-platform.

Useful modes:

```text
--validate-only             validate schema and complete tests/lua coverage
--suite non-official        run the 101 non-official manifest records (default)
--suite differential        run the dedicated four-case differential lane
--suite official            run official-suite records
--suite all                 run every manifest record
--case <id>                 select one exact case ID (repeatable)
--rust-only                 skip the C++ differential
--include-helpers           execute helper/module files intentionally
--allow-differences         report differences without a non-zero runner exit
```

`--allow-differences` applies only to completed semantic comparisons. Manifest
errors, process spawn/capture errors, and timeouts remain non-zero
infrastructure failures.

The manifest contains the original 125 fixtures plus any subsequently added
compatibility fixtures. Its default `non-official` lane intentionally remains
the original 101-record corpus; the four focused probes live in the separate
`differential` lane.

The audited inventory is:

- 20 `entry` records (16 from the M0 baseline and four differential probes);
- 32 `helper` records (eight Alien Signals modules, one integration chunk, and
  23 files driven by official `all.lua`);
- 76 `manual-output` records that still require byte-for-byte differential
  review until assertions or goldens replace visual inspection;
- one `negative` syntax-error record with expected exit status 1.

`helper` records are present in artifacts but skipped unless
`--include-helpers` is set. `negative` records pass only when their declared
non-zero exit status is observed. `manual-output` records are still compared;
the classification means their output has not yet been converted to assertions
or a checked-in golden file.

The report preserves stdout and stderr as hexadecimal bytes as well as lossy
display text. Comparisons use the bytes. Normalization is opt-in per fixture and
is recorded in the manifest; literal normalizations can use
`{{executable}}`, `{{working_directory}}`, `{{repo_root}}`,
`{{script_path}}`, and `{{temp_directory}}`.
Each path placeholder also has an explicit forward-slash form, such as
`{{executable_slash}}`; this is useful when a CLI renders Windows paths with
portable separators.

Each child is placed in its own process group (a Job Object on Windows).
Timeout cleanup therefore terminates descendants as well as the interpreter,
so a fixture that starts a pipe or subprocess cannot leave the runner blocked
on inherited output handles.
