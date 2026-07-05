# Rust Unit Tests

This directory mirrors the role of `lua_cpp/tests/unit` for Rust-side test cases.

Use it for project-level Rust unit test inputs, shared helpers, or module-specific
test organization that should live outside Lua source compatibility tests.
Lua scripts copied from `lua_cpp/tests/lua` live in `../lua`.

Crate-local Rust unit tests should usually stay next to the implementation under
`crates/*/src/**` with `#[cfg(test)]`. Cross-crate integration tests should live
under the relevant crate's `tests/` directory or be wired from a top-level Cargo
test target.
