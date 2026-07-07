---
status: completed
phase: 0
phase_name: Project Infrastructure
date: 2026-06-13
last_updated: 2026-07-07
applies_to: Workspace infrastructure initialization
---

# Phase 0 Report: Project Infrastructure

## Status

Phase 0 established the Rust workspace, CI pipeline, automation scripts,
documentation scaffold, and test directories for the Lua 5.1.5 interpreter.

## Files Created

### Workspace Root

| File | Purpose |
|---|---|
| `Cargo.toml` | Workspace root with 6 member crates, edition 2024, workspace lints. |
| `rust-toolchain.toml` | Rust stable channel with rustfmt, clippy, rust-docs. |
| `.cargo/config.toml` | Windows build config, stack size, and convenience aliases. |
| `.github/workflows/ci.yml` | CI pipeline: format, clippy, build, test, doc, audit, quality gate. |

### Crates

| Crate | Type | Responsibility |
|---|---|---|
| `lua_core` | lib | Runtime value system, GC, strings, tables, functions, userdata, threads. |
| `lua_compiler` | lib | Lexer, parser, AST, opcode metadata, bytecode generation. |
| `lua_vm` | lib | LuaState, stack, call frames, opcode dispatch, runtime execution. |
| `lua_stdlib` | lib | Lua 5.1 standard libraries. |
| `lua_app` | bin | Command-line runner and REPL. |
| `lua_bytecode` | bin | Text/JSON bytecode dump tool. |

### Automation Scripts

| Script | Purpose |
|---|---|
| `tools/rust_env_check.ps1` | Verify Rust toolchain and workspace structure. |
| `tools/rust_quality_gate.ps1` | Run fmt, clippy, tests, docs, and security audit. |

### Documentation

| File | Purpose |
|---|---|
| `docs/glossary.md` | Lua and project terminology glossary. |
| `docs/rust_migration/type_mapping_table.md` | Rust internal type reference. |
| `docs/rust_migration/deviation_log.md` | Compatibility and implementation notes log. |
| `docs/rust_migration/phase_0_report.md` | This report. |

### Test Directories

```text
tests/lua/
tests/unit/
```

Lua compatibility inputs are kept under `tests/lua/` and grouped by behavior area.
Project-level Rust test helpers and supporting inputs live under `tests/unit/`.

## Validation Results

The initial infrastructure validation covered:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_env_check.ps1
cargo build --workspace
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo doc --no-deps
powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_quality_gate.ps1 -SkipAudit
```

All required workspace crates were present, formatting and clippy checks passed,
and documentation generation succeeded.

## Architecture Baseline

The workspace was split around clear ownership boundaries:

- `lua_core` owns runtime data structures and GC.
- `lua_compiler` owns source-to-`Proto` compilation.
- `lua_vm` owns execution state and bytecode dispatch.
- `lua_stdlib` owns Lua standard library registration and host functions.
- `lua_app` and `lua_bytecode` provide user-facing tools.

Workspace-level lints enforce unsafe discipline:

```toml
[workspace.lints.rust]
unsafe_op_in_unsafe_fn = "deny"

[workspace.lints.clippy]
undocumented_unsafe_blocks = "deny"
```

## Follow-Up

The infrastructure is ready for runtime, compiler, VM, standard library, and CLI
work to evolve independently while sharing one quality gate.
