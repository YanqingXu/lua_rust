---
status: accepted-for-migration
last_updated: 2026-07-26
scope: M1.1 ByteString design and M1.2-M1.3 migration contract
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# RFC: Lua ByteString

## Decision

Lua string values are immutable, arbitrary byte sequences. They are not Rust
text and must not acquire a UTF-8 invariant at any internal runtime boundary.
The foundational Rust representation is `ByteString`, backed by a
`Box<[u8]>` containing the logical bytes followed by one NUL sentinel.

The sentinel is not part of equality, ordering, hashing, or length. Embedded
NUL bytes are valid logical data. `as_ptr()` and `len()` describe the same
stable allocation, and the byte at `as_ptr().add(len())` is zero.

The compatibility target for project behavior is fixed at
`lua_cpp@87c15e69ceb94eb74e28226ccbefb7e196635711`. Stock Lua 5.1.5 remains the
language oracle where the project does not intentionally differ.

## Public contract

`ByteString` exposes deliberately narrow operations:

- `from_bytes` accepts any bytes, including empty input, embedded NUL, high
  bytes, and invalid UTF-8.
- `from_utf8_text` is an explicitly named convenience at a known text
  boundary. It copies the UTF-8 encoding without conversion.
- `from_static_ascii` is for keywords and protocol constants and rejects a
  non-ASCII constant as a programming error.
- `as_bytes`, `len`, and `is_empty` describe only the logical bytes.
- `as_ptr` exposes the stable allocation; consumers must always carry `len`
  because an embedded NUL does not terminate a Lua string.
- `to_utf8` is a checked view and `to_string_lossy` is an explicit
  presentation conversion.

There is intentionally no `Display`, `Deref<Target = str>`, or ambiguous
`From<&str>` implementation. These conveniences would make an accidental text
assumption too easy. `Debug` is byte-oriented and lossless.

`Eq`, `Hash`, and `Ord` operate on the logical byte slice. `AsRef<[u8]>` and
`Borrow<[u8]>` have the same logical-byte semantics so borrowed collection
lookups remain valid.

## Hashing policy

Two distinct hash contracts must not be conflated:

1. `ByteString` implements Rust's `Hash` trait by forwarding the complete
   logical byte slice to the caller's `Hasher`. This is the contract required
   by Rust collections and `Borrow<[u8]>`.
2. Lua string interning uses a dedicated Lua-compatible precomputed hash in
   `GcString`/`StringPool`. That hash is not part of `ByteString`.

During M1.2, the dedicated interning hash will be compared byte-for-byte
against stock Lua 5.1.5 and the fixed C++ oracle. If their hash behavior
conflicts, this project follows
`lua_cpp@87c15e69ceb94eb74e28226ccbefb7e196635711`; the conflict and corpus
evidence must be recorded in the deviation log. This choice does not change
the standard Rust `Hash` implementation on `ByteString`.

## Text, display, and path boundaries

UTF-8 validation or lossy conversion is permitted only when bytes cross a
human-facing text boundary, such as diagnostics, REPL output, logs, or the
text mode of a developer tool. Those call sites must choose `to_utf8` or
`to_string_lossy` explicitly. Serialization, IO, pattern matching, string
library operations, bytecode, and C API pointer-plus-length access remain
byte-preserving.

An operating-system path is not a Lua string type. Loader and CLI code must
convert between a Lua byte string and `std::path::Path` at an explicit policy
boundary; core values must not store paths, and path conversion failure must
not silently rewrite the Lua bytes. Platform-specific path policy belongs to
the host/service layer and its compatibility tests.

## Migration sequence

The migration is staged to keep representation changes reviewable:

1. Introduce and test `ByteString` without changing existing runtime users.
2. Change `GcString` payloads and `StringPool` keys to `ByteString`; introduce
   the dedicated oracle-compatible Lua hash and prove interning identity.
3. Convert `Value` consumers and VM/string operations from `&str` shims to
   byte slices.
4. Change lexer/parser and source loading to a byte cursor. Lua grammar tokens
   may use ASCII recognition, while string literal contents remain exact.
5. Convert standard libraries, IO, package loading, dump/load, CLI, and
   bytecode tooling. Keep path and presentation conversion at named edges.
6. Remove the Latin-1/UTF-8 compatibility shims only after no caller depends
   on them and the differential corpus is green.

No temporary dual representation may become a public compatibility promise.
GC ownership, automatic sweeping, and the C API are separate milestones and
must not be folded into this representation change.

## Acceptance criteria

The ByteString primitive is accepted when tests prove:

- empty strings, embedded NUL, high bytes, invalid UTF-8, and every byte from
  `0x00` through `0xff` round-trip unchanged;
- pointer, logical length, and trailing sentinel describe one stable buffer;
- equality, standard hashing, ordering, `AsRef`, and `Borrow` use logical
  bytes only;
- checked UTF-8, lossy presentation, and byte-oriented `Debug` are explicit.

The overall M1.1-M1.3 migration is complete only when:

- `GcString` and `StringPool` intern arbitrary bytes with oracle-compatible
  hash and identity behavior;
- lexer, VM, libraries, IO, dump/load, CLI, and future C API preserve bytes;
- UTF-8 source and values are not double-encoded;
- `#string.char(255)`, embedded NUL, long-string hashing, IO round-trips, and
  pointer-plus-length differential cases match both applicable oracles;
- legacy text/Latin-1 shims are removed and
  [NOTE-007](deviation_log.md#note-007-lua-字符串尚未采用任意字节表示) can be
  closed with evidence.
