# Initial machine-readable oracle baseline

- Date: 2026-07-26
- Change type: initial baseline lock
- Required review label: `oracle-baseline-change`

## Locked references

- Project oracle: `https://github.com/YanqingXu/lua.git` at
  `87c15e69ceb94eb74e28226ccbefb7e196635711`.
- Language oracle: Lua 5.1.5 source archive with SHA-256
  `2640fc56a795f29d28ef15e13c34a47e223960b0240e8cb0a82d9b0738695333`.
- Official test archive: `lua5.1-tests.tar.gz` with SHA-256
  `49e4ca6561f82ea605908c5041ab5fad66ed9930fa0686675bd51b02767f18ad`.

This change does not move an existing oracle. It turns the baselines recorded in
`plan.md` into machine-readable configuration and adds a fail-closed validator.

## Observable comparison changes

- Adds the four C++ differential probes: value types, arithmetic error category,
  weak-value collection, and split stdout/stderr.
- Captures raw stdout and stderr bytes, their SHA-256 values, Base64, exit status,
  timeout, and UTF-8 diagnostic views.
- Permits only the manifest-declared CRLF-to-LF comparison normalization; raw
  bytes remain in the report.
- Records the stock Lua `_VERSION` difference through `NOTE-001`. The candidate
  intentionally follows the fixed C++ project oracle value.

## Initial evidence

- Oracle metadata and the 24 currently vendored Lua suite files validate against
  the canonical Git blobs.
- Rust versus C++: four of four probes match after the declared line-ending
  normalization; `_VERSION` matches exactly.
- Rust versus official Lua 5.1.5: four of four probes match; the `_VERSION`
  project identity is the sole accepted, precisely matched deviation.

The nine non-Lua support files from the upstream official suite are listed in
the source manifest. Until they are vendored, the validator reports the
`tracked-subset` policy explicitly rather than treating the suite as complete.
