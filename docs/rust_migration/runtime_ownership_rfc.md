---
status: accepted-design
implementation_status: incomplete
schema_version: 1
last_updated: 2026-07-29
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# Runtime ownership and GC lifecycle RFC

## 1. Decision

`lua_rust` will use one explicit `Runtime` as the lifetime owner of every VM
service. `Runtime` owns one `Heap`, one `StateArena`, one root registry, and the
runtime-scoped host services. `Heap` owns the allocator/accounting layer,
`StringPool`, and `GarbageCollector` as one destruction unit.

This document and
[`gc_root_inventory.json`](../../tests/compatibility/gc_root_inventory.json)
are design and audit artifacts. M1.7 mark-only tracing and the M1.8 partial
shutdown substrate described below are implemented, but their presence does
**not** mean live-VM collection or complete Lua shutdown semantics are enabled.

The current public Lua `collectgarbage("collect")` path must not be connected
to a destructive sweep until every prerequisite in section 10 is green.
Low-level mark/sweep unit tests are not evidence that a live VM can be swept
safely.

## 2. Why this is a hard prerequisite

The audited implementation has useful GC components, but the lifetime contract
is not closed:

- `GarbageCollector` stores an intrusive raw-pointer list and raw-pointer work
  queues in `crates/lua_core/src/gc/collector.rs:22-59`.
- allocation transfers `Box` ownership into a raw pointer at
  `crates/lua_core/src/gc/collector.rs:87-107`;
- normal `GarbageCollector::drop` deliberately unlinks without freeing objects
  at `crates/lua_core/src/gc/collector.rs:405-422`;
- `clear_all` keeps fixed objects at
  `crates/lua_core/src/gc/collector.rs:169-208`;
- `StateArena` now owns coroutine `Box<LuaState>` allocations and reconstructs
  them during removal/shutdown; `Thread` stores and traces a validated
  `StateHandle`. The main state is still an externally owned arena slot.
  Coroutine resume/wrap now transfer execution through the Runtime activation
  trampoline after releasing the caller state borrow;
- coroutine/debug lookup is scoped through `with_resolved_state_mut`; open
  Upvalues now identify their owner with `StateHandle + stack index`, owner
  states keep the non-intrusive ordered collection, and Runtime turns schedule
  ordinary cross-state GET/SET without overlapping state borrows;
- M1.6 removed the dump shim that kept unrooted `GcRef<Proto>` values in
  thread-local maps; `string.dump` now fails explicitly until the M3
  serializer exists, and source loaders no longer recognize the private
  pseudo-dump prefix;
- `Runtime::trace_roots_mark_only` now provides the canonical object/state
  fixed-point root callback, but temporary state roots, pending finalizers,
  fixed strings, and several production publication paths remain incomplete;
- the two VM-side weak-cleanup scanners at
  `crates/lua_vm/src/execute.rs:1137-1188` and
  `crates/lua_stdlib/src/base.rs:432-495` disagree and are not safe sweep root
  scanners;
- `GcRef<T>` now carries a process-unique, non-reused `ObjectId`, and each
  collector has an authoritative address-to-identity/type live table. This
  closes pointer-address reuse for checked collector entry points, but does
  not yet provide the final unique `Heap` owner or lexical temporary roots.
- safe `Value::String` equality and hashing still dereference the string
  candidate without a collector-side validation context. Production code
  currently creates non-interned `GcString` values directly, so changing Lua
  string equality to handle identity would be a semantic regression. This is
  an explicit sweep blocker, not a completed provenance claim.

Standalone collector Drop and incomplete live-collection paths still use
leaking as a fail-safe for several dangling-reference defects. Runtime close
now has an explicit shutdown-only reclamation path; wiring the same destruction
into a live sweep before closing the remaining ownership/root contracts would
still turn those defects into use-after-free.

## 3. Oracle contract

The lifecycle oracle is `lua_cpp@87c15e69ceb94eb74e28226ccbefb7e196635711`.
The Rust design need not reproduce C++ types, but it must reproduce the
observable and ownership contracts:

- `RuntimeServices` is a borrowed service bundle, while `EngineContext` is the
  unique non-copyable owner:
  `lua_cpp/src/runtime/runtime_services.hpp:22-97`;
- `EngineContext` owns allocator, strings, and global state in an order that
  leaves dependencies alive for teardown:
  `lua_cpp/src/runtime/runtime_services.hpp:192-195`;
- `GlobalState` owns registry, main/running state records, primitive
  metatables, and fixed strings:
  `lua_cpp/src/vm/state/global_state.hpp:222-249`,
  `lua_cpp/src/vm/state/global_state.hpp:283-347`, and
  `lua_cpp/src/vm/state/global_state.hpp:370-455`;
- `GlobalState::markRoots` traces the complete runtime-level root set:
  `lua_cpp/src/vm/state/global_state.cpp:162-195`;
- a `Thread` owns or refers to its `LuaState` with an explicit owner type and
  traces its state/caller graph:
  `lua_cpp/src/core/thread.hpp:27-68`,
  `lua_cpp/src/core/thread.hpp:117-128`, and
  `lua_cpp/src/core/thread.cpp:487-499`;
- `Thread::resume` rejects only `Dead` and `Running`, changes its caller from
  `Running` to `Normal`, and therefore deliberately permits a `Normal`
  ancestor to be resumed:
  `lua_cpp/src/core/thread.cpp:84-383`;
- GC objects record their collector owner:
  `lua_cpp/src/core/gc_object.hpp:159-164` and
  `lua_cpp/src/core/gc_object.hpp:199-249`;
- collector destruction frees ordinary and fixed objects, and bulk shutdown
  destroys threads before the remaining object graph:
  `lua_cpp/src/gc/garbage_collector.cpp:102-118` and
  `lua_cpp/src/gc/garbage_collector.cpp:933-988`;
- close rejects foreign-thread and busy-state teardown before finalizing:
  `lua_cpp/src/api/lapi.cpp:679-704`.

The matching C++ isolation and owner-thread tests are in
`lua_cpp/tests/unit/vm/test_runtime_services.cpp:136-225` and
`lua_cpp/tests/unit/vm/test_runtime_services.cpp:468-545`.

## 4. Ownership model

The target ownership tree is:

```text
Runtime
├── RuntimeId + owner ThreadId + RuntimePhase + !Send/!Sync marker
├── RuntimeServices
│   ├── native/module services
│   ├── IO/resource services
│   └── execution/compilation policy
├── GlobalRoots
│   ├── global table and persistent registry
│   ├── primitive metatables and fixed strings
│   └── main/running state or thread handles
├── StateArena
│   ├── main LuaState slot
│   └── coroutine LuaState slots
└── Heap
    ├── HeapId
    ├── allocator and live/peak accounting
    ├── StringPool
    └── GarbageCollector
```

The dependency arrows are:

```text
Runtime/Heap owner
        │
        ├──> StateArena + generational StateHandle
        │
        ├──> runtime-scoped registry/dump policy
        │
        └──> one authoritative RootTracer + temporary roots
                         │
                         └──> deterministic shutdown
                                      │
                                      └──> internal stop-the-world sweep
                                                   │
                                                   ├──> weak/finalizer semantics
                                                   └──> mutation barriers
                                                               │
                                                               └──> incremental GC
```

No arrow may be skipped. In particular, adding a call to `sweep` is not an
acceptable shortcut for any earlier node.

### 4.1 `Runtime`

The conceptual API is:

```rust,ignore
pub struct Runtime {
    id: RuntimeId,
    owner_thread: ThreadId,
    phase: RuntimePhase,
    services: RuntimeServices,
    roots: GlobalRoots,
    states: StateArena,
    heap: Heap,
    active_execution: Vec<StateHandle>,
    not_send_or_sync: PhantomData<Rc<()>>,
}
```

Requirements:

1. `Runtime` is the only public owner that can start collection or shutdown.
2. It is neither `Send` nor `Sync`. A separately designed atomic cancellation
   handle may be `Send`.
3. every mutable entry point checks the owner thread before reading or
   modifying runtime state;
4. phases are at least `Running`, `Closing`, and `Closed`;
5. only one main state may be installed at a time;
6. `try_close` is explicit, checked, and idempotent with respect to partial
   failure; `Drop` is only a final safety net;
7. an operation that observes `Closing` or `Closed` cannot allocate, resume a
   coroutine, or begin a collection.

### 4.2 `Heap`

`Heap` is defined below the VM layer so the compiler and core object code can
use it without depending on `lua_vm`:

```rust,ignore
pub struct Heap {
    id: HeapId,
    allocator: LuaAllocator,
    strings: StringPool,
    collector: GarbageCollector,
    stats: HeapStats,
}
```

Requirements:

1. `GarbageCollector::new`, destructive collection, and raw object access
   become internal implementation details;
2. string interning and object allocation go through `Heap`, so they cannot be
   paired with a different collector;
3. every managed object records or can be validated against `HeapId`;
4. every allocation has one matching destructor and deallocation route;
5. object count, GC-accounted bytes, allocator live bytes, and allocator peak
   bytes are separate metrics;
6. container growth/shrink updates the object's accounted size. Subtracting a
   newly computed size from an old allocation estimate is forbidden;
7. `Heap::destroy_all` deletes normal and fixed objects and leaves every work
   queue empty;
8. `Heap::Drop` can finish cleanup while its string pool and allocator are
   still alive. A standalone collector destructor may not depend on an already
   destroyed external pool.

### 4.3 `StateArena` and `StateHandle`

The target handle is conceptually:

```rust,ignore
pub struct StateHandle {
    runtime_id: RuntimeId,
    slot: usize,
    generation: u64,
}
```

The handle representation belongs in a dependency-neutral module; the arena
belongs to `lua_vm`. The implemented issuance boundary is a non-`Clone`,
non-`Copy` `StateHandleIssuer`: its checked process-global allocator reserves
zero as invalid and `u64::MAX` as the permanent exhausted sentinel, and only
issues `1..=u64::MAX - 1`. An issuer can create handles only in its own fresh
namespace; safe code cannot rebuild an existing `RuntimeId` or a handle from
raw integers. `Runtime::try_new` reports terminal identity exhaustion before
allocating a heap, while `Runtime::new` fails closed by panicking.

Requirements:

1. the arena is the sole owner of every `LuaState`;
2. `Thread` stores a `StateHandle`, never `*mut c_void` or `Box<LuaState>`;
3. a slot generation changes before reuse and never wraps: generation
   `u64::MAX` may be issued once, then its vacant slot is permanently retired;
4. runtime id, slot bounds, generation, and occupancy are checked on every
   lookup;
5. a stale or foreign handle produces a deterministic runtime error;
6. no API returns `&'static LuaState`, `&'static mut LuaState`, or another
   reference whose lifetime is detached from an arena borrow;
7. the active execution stack records which states are busy and is also a GC
   root/busy-close signal;
8. coroutine execution is driven as Runtime-owned turns: the current
   `LuaState` borrow is dropped before an owned switch request is processed or
   the next state is borrowed. A scheduler frame may retain handles and rooted
   values, but never a state reference;
9. an unreachable `Thread` causes its state to close before the thread and
   other GC objects are freed;
10. open upvalues identify their owner by `StateHandle + stack index`, not by
    a pointer to a movable `Stack`;
11. retired, occupied, borrowed, and duplicate free-list entries are never
    reused; close validates arena counts and free-list invariants before any
    destructive mutation;
12. every fallible free-list capacity operation happens before clearing an
    owned state pointer, so unwind cannot orphan its Box.

Arena membership alone is not reachability. The main/running state is traced
from `GlobalRoots`; another state is traced only after a reachable `Thread`
enqueues its handle.

#### 4.3.1 Coroutine activation, including `Normal` ancestor re-entry

The pinned C++ target and stock Lua 5.1.5 intentionally differ at one critical
state-machine edge. The focused fixture
`tests/characterization/coroutine-normal-ancestor.lua` runs `A -> B -> A`:
while B is running, A is its `Normal` ancestor. The checked characterization
artifact and replay tool are:

- `tests/compatibility/coroutine-normal-ancestor-characterization.json`;
- `tools/check_coroutine_normal_ancestor_characterization.ps1`.

The C++ target accepts the second resume, executes A's pending continuation,
returns A's result to B, and later executes the same A continuation again when
the original A-to-B activation unwinds. Stock Lua rejects the second resume as
`cannot resume normal coroutine` and completes in ordinary LIFO order. Three
repetitions lock the normalized C++ stdout at SHA-256
`bad37c42fcfd369f22fdc9d9ec8d1ce46caaa2e8fa755fe03a41a6e91b2591d2`
and the stock stdout at
`0488432cb01117da75f229ab0f43bd1c1ea174853ebde5f1ab62853265f805f6`.
This is a non-gating characterization while Rust remains incomplete; it is
not an approved deviation from the project target. The replay tool supports
Windows PowerShell 5.1 and PowerShell 7, consumes the existing C++ and stock
Lua build-provenance reports, and rejects an executable whose SHA-256 does not
match the reported build.

Consequences for the trampoline design:

1. scheduling state is an activation stack, not a set of unique active
   `StateHandle` values. The same state may occur in more than one activation;
2. `Suspended` and an active-chain `Normal` target are resumable under the C++
   contract. `Running` and `Dead` remain errors. A `Normal` target outside the
   active chain is an internal invariant failure;
3. every activation frame owns its caller/callee handles, prior statuses and
   caller link, resume-versus-wrap envelope, exact result destination,
   continuation/PC, execution-count state, and rooted transfer payload;
4. the Runtime borrows at most one state during a turn. A resume callback
   publishes an owned request, returns from the VM turn, and releases the
   caller borrow before the target is resolved—even when the target is an
   ancestor;
5. no safety shortcut may silently reject all `Normal` targets. If the project
   later chooses stock behavior instead, that choice requires a separately
   approved deviation with an exact differential expectation.

### 4.4 `VmContext`

VM, compiler, and standard-library operations receive short-lived capability
borrows:

```rust,ignore
pub struct VmContext<'r> {
    runtime_id: RuntimeId,
    state: &'r mut LuaState,
    heap: &'r mut Heap,
    roots: &'r mut GlobalRoots,
    services: &'r mut RuntimeServices,
}
```

The concrete split may differ, but it must preserve these properties:

- no raw `GarbageCollector` or `StringPool` pointer is stored in `LuaState`;
- callbacks reconstruct scoped borrows at the callback boundary and never
  manufacture a `'static` reference;
- the compiler receives `&mut Heap` or a narrower compiler service, replacing
  the raw pointers and unsafe `Send`/`Sync` implementations in
  `crates/lua_compiler/src/codegen/mod.rs:87-136` and
  `crates/lua_compiler/src/codegen/builder.rs:22-47`;
- mutation goes through a `MutationContext`/`VmContext` method, so barrier,
  provenance, and accounting checks cannot be bypassed by an ordinary setter.

### 4.5 GC handles and temporary roots

`GcRef<T>` cannot remain a freely dereferenceable, pointer-only public
capability once sweep is enabled. The preferred implementation is a
generational object handle. A transitional pointer representation is allowed
only if all of the following hold:

1. it carries `HeapId` and a generation;
2. validation consults a live allocation table before dereferencing memory;
3. ordinary safe `Eq`, `Hash`, and formatting do not dereference a possibly
   stale pointer;
4. cross-heap edges are rejected before publication;
5. object access requires a scoped `Heap`/`VmContext` borrow;
6. no unvalidated handle can escape through a public safe API.

Any handle held across an operation that may advance GC must be protected.
The API must provide a lexical `RootScope`, `Rooted<T>`, or equivalent. A
builder may use a checked `NoGcGuard` only when it defers collection and pays
the accumulated debt immediately after publication.

Known temporary-root hazards include:

- the Proto-to-Function publication gap at
  `crates/lua_app/src/main.rs:338-341`;
- the unregistered IO table/userdata construction graph at
  `crates/lua_stdlib/src/io.rs:116-182`;
- compiler-owned, not-yet-registered Proto graphs containing GC strings and
  child protos.

The default allocation API should return a protected new object or require an
explicit publication transaction; relying on callers to remember an
untracked root is not fail-closed.

The implemented P1 provenance slice is intentionally narrower than that final
API:

- `Value::LightUserdata` uses a separate one-pointer `LightUserdataRef`; an
  arbitrary host pointer can no longer be constructed as a managed `GcRef`.
- every registered allocation receives a process-global monotonic
  `ObjectId(u64)`; zero is reserved for null and exhaustion panics before
  wraparound or reuse;
- `GcRef<T>` carries `(candidate pointer, ObjectId)`, while collector roots
  retain a type-erased `(pointer, ObjectId, tag)` handle;
- `GarbageCollector::validate_ref`, `with_ref`, `with_mut`, and
  `mark_registered` consult the collector's address-to-`(ObjectId, tag)` side
  table before object memory is read;
- root seeding, child tracing, write barriers, weak/dead checks, and finalizer
  discovery reject foreign, stale, address-reused, or type-confused handles
  fail-closed;
- weak-table, pending-finalizer, and external-mark queues retain typed or
  erased handles including `ObjectId`, so a persistent queue cannot
  re-identify a later allocation at the same address;
- `StringPool` validates a cached handle against the supplied collector before
  reuse, evicts stale/foreign entries by owned byte key, and removes entries
  by allocation identity without dereferencing the candidate;
- gray work items remain raw headers only as an internal queue. They are
  checked against the live table before dispatch and are pruned before an
  allocation is dropped.

This does **not** authorize live sweep. In particular, `GcRef::as_ref` remains
an unsafe transitional escape hatch used by VM code, production construction
does not yet force all strings through one interning boundary, and safe
`Value::String::{eq, hash}` must preserve byte-content semantics but currently
lacks a collector borrow. Until those call sites use checked scoped access (or
string identity becomes a proven invariant), a stale string handle can still
make safe equality/hashing read reclaimed memory. `ConstantKey` already hashes
string handles by allocation identity, and safe `Value` formatting reports
only pointer identity, but those narrower fixes do not close the string debt.

## 5. Root tracing contract

The normative inventory is
[`tests/compatibility/gc_root_inventory.json`](../../tests/compatibility/gc_root_inventory.json).
It is validated by
[`tools/check_gc_root_inventory.ps1`](../../tools/check_gc_root_inventory.ps1).

There is one root traversal implementation. The weak-table maintenance path,
full collector, incremental collector, shutdown diagnostics, and tests all use
the same tracer.

The tracer maintains two work queues:

1. a GC-object gray queue;
2. a `StateHandle` queue.

Tracing reaches a fixed point:

```text
seed GlobalRoots and temporary roots
while either queue has work:
    trace one GC object
      Thread edges may enqueue StateHandle values
    trace one state
      state roots may enqueue GC objects
```

At minimum, state tracing covers the active stack window, active `CallInfo`
function slots and varargs, active Proto/function ownership, open upvalues,
debug hook state, yielded values, last error, and state environments. Full and
incremental collection use a conservative wide active-register window unless
a proven complete stack map exists. Retired stack slots are set to nil so they
cannot accidentally retain or expose stale handles.

`pending_finalizers` is an internal root until each queued finalizer has
finished. Every destruction path removes an object from every queue before
freeing it.

## 6. Mutation and incremental contract

Stop-the-world collection may be implemented before incremental collection,
but incremental collection may not begin until every edge publication path
uses a barrier.

The mutation audit includes:

- table key, value, array, and metatable writes:
  `crates/lua_core/src/table.rs:127-166`,
  `crates/lua_core/src/table.rs:252-272`, and
  `crates/lua_core/src/table.rs:439-442`;
- Function Proto/upvalue/environment publication:
  `crates/lua_core/src/function.rs:182-209`;
- Upvalue close, set, and open-list publication:
  `crates/lua_core/src/upvalue.rs:113-178`;
- Userdata metatable/environment publication:
  `crates/lua_core/src/userdata.rs:197-200`;
- Thread caller and state-handle publication:
  `crates/lua_core/src/thread.rs:189-206`;
- Proto source/constants/subprotos/debug names:
  `crates/lua_core/src/proto.rs:327-382`,
  `crates/lua_core/src/proto.rs:483-516`, and
  `crates/lua_core/src/proto.rs:562-564`;
- global, registry, metatable, stack, debug, yielded-value, and error root
  assignments.

The existing barrier helper at `crates/lua_core/src/gc/mark.rs:308-377` is a
component, not a completed contract: production mutation paths do not call it,
it does not validate heap ownership, and it has no complete phase policy.

Incremental phases are `Pause`, `Propagate`, `Atomic`, `Sweep`, and `Finalize`.
The initial snapshot and atomic boundary rescan wide stack roots. New
allocations during an active cycle publish their complete initial graph and
cannot be swept in that cycle. `step`, pause, multiplier, debt, stop, and
restart operate on collector state rather than simulated counters in
`LuaState`.

## 7. Deterministic close

`Runtime::try_close` follows this order:

1. verify owner thread and reject a foreign close;
2. reject a busy runtime without mutating it;
3. transition `Running -> Closing` and stop automatic collection/allocation
   entry;
4. close open upvalues while their state stacks and Upvalue objects are alive;
5. drain remaining `__gc` callbacks in protected calls, isolating callback
   errors and keeping the pending queue rooted;
6. clear native/module/IO service roots while those services are still alive;
7. clear global, registry, primitive-metatable, fixed-string, running-thread,
   and temporary roots;
8. remove all `LuaState` arena slots and advance or permanently retire their
   generations without wrap;
9. destroy Thread objects first, then all other ordinary objects, then fixed
   objects;
10. clear string-pool indexes and collector work queues;
11. verify state count, object count, accounted bytes, allocator live bytes,
    and pending-finalizer count are zero;
12. transition to `Closed`.

The implementation uses an explicit phase and owned `Option::take`/dedicated
teardown methods rather than relying on implicit Rust field-drop order.
Shutdown is safe after a partially constructed runtime and after a protected
finalizer error.

`Drop` must not silently leak on the normal owner-thread path. If a foreign
thread can obtain a raw host handle despite `!Send`, the explicit close API
returns an owner error; the policy for an impossible foreign-thread `Drop`
must be documented and tested (the C++ oracle terminates).

## 8. Phased implementation

Each letter is intended to be an independently reviewable PR. A later phase
does not redefine an earlier phase's invariants.

### A. Runtime shell — M1.4

- add `Heap`, `Runtime`, `RuntimeId`, `HeapId`, `GlobalRoots`, runtime phase,
  owner-thread check, and `!Send`/`!Sync` marker;
- create a persistent per-runtime registry;
- migrate CLI construction and compiler/VM/stdlib service access to scoped
  contexts;
- remove stored raw GC/pool pointers and unsafe compiler `Send`/`Sync`;
- keep destructive VM sweep disabled.

Acceptance: two runtimes have distinct heaps, string pools, registries, and
handles; a second main state and foreign handle are rejected.

### B. State arena — M1.5

- add generational `StateArena`/`StateHandle`;
- move main and coroutine states into it;
- replace `Thread.state` and `caller_state` raw pointers;
- replace open-upvalue stack pointers with state handle plus slot;
- remove all runtime `&'static`/`&'static mut` pointer helpers.

Acceptance: stale slot reuse and cross-runtime lookup fail; 1,000
create/release coroutine cycles return the live-state count to baseline.

#### B.1 Implemented identity/generation substrate

The StateHandle issuance/exhaustion slice is complete locally:

- Runtime identity allocation is atomic, monotonic, non-zero, concurrent-safe,
  and permanently returns `RuntimeIdExhausted` at its reserved sentinel;
- the arena privately owns the only issuer for its runtime namespace;
  compile-fail tests reject raw `RuntimeId`/`StateHandle` construction and
  issuer cloning;
- ordinary vacate advances with `checked_add`; the final generation is issued
  once and the next vacate marks the slot retired without placing it on the
  free-list;
- stale/foreign/retired handles are rejected before dereference, malformed or
  duplicate free-list entries cannot overwrite occupied slots, and shutdown
  preflights arena invariants before mutation;
- concurrent allocation, 1,000 reuse cycles, MAX-generation reuse/retirement,
  corrupted free-list, real stale/foreign tracing, and MAX-generation close
  have focused regressions.

Phase B remains incomplete because main-state ownership and the
debug/protected-helper cross-state scheduling matrix are still outstanding.

#### B.2 Implemented Runtime turn-borrow substrate (partial)

`Runtime::drive_state_turns` and `RuntimeTurn::{Switch, Complete}` now provide
a crate-private ownership substrate for the future coroutine driver. One
session counts as one active Runtime execution, but it does not keep a
`LuaState` reference between turns. Each iteration:

1. validates and borrows exactly one `StateHandle`;
2. confines the state, collector, and string-pool references to a
   higher-ranked callback;
3. receives an owned `Switch` or `Complete` outcome;
4. drops the state guard before resolving the next handle.

While a Runtime turn is active, nested `with_resolved_state_mut` and direct
owner borrows fail with `TurnBorrowActive`. Starting a turn also rejects any
pre-existing borrowed slot. Test-only acquisition/release instrumentation
proves the exact `main -> child -> main` event order and a peak of one borrowed
slot. Panic unwind releases the turn marker, state borrow, and active-execution
count; foreign, stale, and already-borrowed handles fail closed; a released
handle can be selected again in a later turn. These tests increased the
workspace total from 733 to 737.

Coroutine creation also now installs `State -> Thread` before inserting the
state, then binds `Thread -> StateHandle` before exposing the Thread value.
This removes its former initialization-only second-state borrow. It does not
replace the still-missing `PendingState` rollback/root transaction.

This substrate is now the ownership foundation used by the production
coroutine trampoline described below.

#### B.3 Implemented Runtime coroutine activation trampoline (local-complete slice)

`coroutine.resume` and the wrap runner are sealed `RuntimeNativeFunction`
operations. They publish a `ResumeRequest` through a scoped mailbox and return
`VmExit::NativeRequest`; they cannot recursively resolve or execute a target
state. The VM seals the suspended native call as a deferred C frame, including
its continuation snapshot and exact result destination.

`Runtime::execute_proto` drives the request with an owned activation stack.
It drops the caller guard before validating and borrowing the target, moves
arguments/results/errors through owned activation buffers, and seeds those
buffers from the canonical root tracer. It restores status, caller links,
yield permission and saved execution counters on unwind. Generic-for
continuations and protected `pcall` boundaries around resume/wrap are also
resumed through the same protocol.

The CLI and stdlib integration harness now enter execution through Runtime
scheduling. Focused process tests cover ordinary yield/resume, protected
resume, wrap yield/error, and the exact pinned-C++ `A -> B -> A` `Normal`
ancestor behavior, including the continuation's second execution. Together
with the sealed-function unit test, this increases the workspace total from
737 to 741.

This is a completed local trampoline slice, not completion of Phase B:

- debug cross-state operations remain outside the request protocol;
- deep-chain and broader fault-injection matrices remain open;
- raw GC/StringPool backpointers and main-state external ownership remain;
- the activation buffer is a canonical root seed, but other partial/missing
  roots still prohibit live destructive collection.

#### B.4 Implemented checked open-Upvalue ownership (local-complete slice)

An open `Upvalue` now stores only `Open { owner: StateHandle, stack_index }`.
`LuaState` owns a non-intrusive `Vec<GcRef<Upvalue>>`, kept unique and sorted by
descending stack index so close-at-level preserves the Lua ordering contract.
Every read, write, close, debug access, root edge, and shutdown edge validates
the collector identity and checked location before touching the stack.

When ordinary bytecode accesses an open Upvalue owned by another suspended
state, the VM returns `VmExit::UpvalueAccess`. The Runtime releases the
requester turn, borrows the owner for one checked read/write, then returns to
the requester to continue execution. The transfer request/response remains in
the Runtime activation root buffer, and a pending native delivery—including
the pinned-C++ `Normal` ancestor replay context—survives any intervening
Upvalue turns. At no point are two state slots borrowed together.

GC propagation from a reachable open Upvalue publishes its owner
`StateHandle`; the canonical two-queue tracer then snapshots the owner state
and marks the indexed stack value even when no reachable `Thread` provides the
state edge. StateArena shutdown closes each validated open Upvalue before
advancing or permanently retiring the owner generation.

Focused tests cover duplicate-slot reuse and ordering, detached-state
rejection, cross-state read/write through a closure yielded by a suspended
coroutine, owner-state root fixed point without a Thread edge, close before
handle invalidation, and exact preservation of the pinned-C++ `A -> B -> A`
characterization. Debug and protected-helper cross-state access remains an
explicit later protocol extension; it no longer uses a raw pointer fallback.

### C. Registry and dump lifetime — M1.6

- completed: delete thread-local `GcRef<Proto>` and pointer-keyed source maps;
- completed: return a stable unsupported error until the real serializer is
  ready, with no private pseudo-dump read path;
- make `debug.getregistry()` return the same per-runtime table.

Acceptance: no static/thread-local GC handles; dump data cannot join two heaps;
registry identity is stable within and isolated across runtimes.

### D. Canonical root tracer — M1.7

- implement the inventory in one tracer;
- add the object/state fixed-point queues;
- add `RootScope`/publication protection;
- make collector sweep unreachable without a `Runtime` root provider;
- first expose mark-only/live-set diagnostics, not destruction.

Acceptance: one survival/removal regression per `RootKind`; the inventory
checker passes.

#### D.1 Implemented mark-only foundation

The first M1.7 slice is implemented as
`Runtime::trace_roots_mark_only` in
`crates/lua_vm/src/runtime/root_trace.rs`. It is deliberately available only
while the Runtime is `Running`, on its owner thread, with
`active_executions == 0`. Holding a `RuntimePartsMut` guard prevents the call in
safe Rust, and the runtime counter is checked again at the API boundary.

The implementation has two independent queues and runs them to a fixed point:

1. `GarbageCollector::begin_mark_only` resets mark colors and validates
   collector explicit roots by live-list membership;
2. the Runtime seeds its global table, persistent registry, and main
   `StateHandle`;
3. one validated state snapshot can enqueue registered GC values;
4. one concrete GC propagation step can enqueue a reachable Thread's state or
   a reachable open Upvalue's owner `StateHandle`;
5. runtime id, slot, generation, occupancy, and borrow state are checked before
   a LuaState pointer is used to copy a snapshot.

Arena membership is not a root. A coroutine state is scanned only after a
reachable Thread or open Upvalue publishes its handle. The state snapshot currently covers the
global table, thread/chunk environments, registry, nil/boolean/number
metatables, exact `0..LuaState::top` stack window, active CallInfo function
slots, managed Proto handles and varargs, every open Upvalue, current Thread,
debug hook and its one-shot managed Proto identity, yielded values, and last
error. `ACTIVE_PROTO` is a direct CallInfo root, including top-level
pseudo-frames that do not have a Function value in their function slot;
`DEBUG_PROTO` is likewise a direct root while line suppression is pending.

`MarkOnlyReport` exposes sorted state successes/failures, inventory-aligned
root-edge counts, marked/total object counts, rejected collector roots,
rejected object-graph child edges, unresolved copied GC edges, remaining gray
work, and explicit unsafe gaps. Every child candidate passed through
`mark_object` is first compared (without dereference) against the collector's
live intrusive list. A foreign collector's otherwise-live `GcRef` is rejected:
cross-collector object graphs are not supported because this Runtime's
collector cannot preserve a foreign allocation's lifetime. Stale and foreign
state handles are diagnostic results, not dereference paths. A regression test
roots a local Table containing another collector's Table and asserts that the
foreign edge is counted, not dereferenced or marked.

Activity Proto ownership no longer uses raw state pointers:

- `CallInfo::proto` is an ObjectId-bearing `GcRef<Proto>`, and
  `LuaState::current_proto` has been removed;
- nested CALL, TAILCALL, generic-for, resume, and host execution paths pass
  the managed handle and reject stale or foreign handles through the owning
  collector before dereference;
- return, error unwind, frame reuse, and pop clear inactive Proto and vararg
  roots;
- `debug_hook_skip_proto` is a managed identity, is traced as `DEBUG_PROTO`,
  and is cleared immediately after its single matching suppression.

This slice still does not claim live sweep is safe. Interpreter and debug
paths validate address, ObjectId, and concrete type immediately before
creating a short transitional reference, but rely on the explicit invariant
that destructive sweep cannot run during VM execution or a debug callback.
Top/stack and active-frame/function-index inconsistencies remain explicit
fail-closed diagnostics. The open-Upvalue root kind no longer contributes an
unsafe-gap variant.

No path in this API calls `collect`, `sweep`, `clear_all`, finalizers, or object
destruction. Existing weak-maintenance scanners now use managed Proto roots
and checked metadata reads, but have not yet been replaced by this Runtime-only
safe-point API. Destructive sweep remains blocked on scoped VM borrows,
unreachable-state integration, finalizer roots, production publication
migration, temporary state roots, and complete finalizer/shutdown semantics.

#### D.2 Implemented temporary-object root foundation (partial)

`GarbageCollector::with_publication` now creates a higher-ranked
`PublicationTxn<'scope>`. `alloc` registers the object and inserts its full
`(pointer, ObjectId, tag)` identity into a collector-owned temporary-root map
before returning a branded `Rooted<'scope, T>`. The higher-ranked closure has a
compile-fail test proving that `Rooted` cannot appear in the return type. Safe
code cannot extract its raw `GcRef`; the initial safe publication operation
installs an explicit collector root before releasing the temporary identity.

Nested transactions retain outer roots. Normal Drop and panic unwind remove
only the exact IDs owned by the transaction. `begin_mark_only` validates and
seeds these roots and reports temporary seeded/rejected counts. Focused tests
cover nested panic, foreign/stale rejection, explicit-root promotion, mark
seeding, and 1,000 scopes returning the registry to zero.

This is an API/registry foundation, not completion of publication safety.
Compiler Proto trees, IO graphs, library registration, VM temporaries,
coroutine `State -> Thread`, app arguments, and returned `Vec<Value>` paths
still use unprotected production allocation. A separate
`TEMPORARY_STATE_ROOTS` inventory entry is therefore `missing`, and
allocation-triggered collection remains disabled.

### E. Shutdown substrate — M1.8

- implement checked, idempotent close phases;
- close states/upvalues before the heap;
- prune all collector queues;
- destroy threads first and fixed objects last;
- run typed Userdata payload destructors exactly once;
- add real live/peak counters.

This phase establishes destruction mechanics. M1.8 is not fully complete until
phase G supplies finalizer drain semantics.

#### E.1 Implemented shutdown substrate (partial)

The C++ audit confirmed that `lua_tryclose` first runs `finalizeAll`, then
destroys the main state; collector teardown subsequently destroys non-fixed
Thread objects, other non-fixed objects, and finally fixed objects. Each
GCString is removed from a still-live StringPool before its allocation is
freed. The relevant oracle paths are
`lua_cpp/src/api/lapi.cpp:675-704`,
`lua_cpp/src/vm/state/lua_state.cpp:357-374,459-466`, and
`lua_cpp/src/gc/garbage_collector.cpp:102-118,852-896,933-988`.

The current Rust partial deliberately omits the C++ `finalizeAll` step and
therefore does not claim complete M1.8 or Lua-compatible close semantics.
`Runtime::close` now performs this non-collecting order:

1. validate owner thread, zero active executions, the main external arena slot,
   and every owned arena slot before mutation;
2. drain owned coroutine states in deterministic slot order, close only
   collector-validated open Upvalues, advance or permanently retire each
   generation without wrap, and drop the state;
3. close validated main-state Upvalues, detach and advance/retire its external
   slot, and drop the main Box;
4. clear Runtime global/registry handles and collector roots;
5. call `GarbageCollector::destroy_all` with the live StringPool;
6. destroy non-fixed Threads, other non-fixed objects, and fixed objects in
   three passes, removing every object from roots, gray, weak, pending
   finalizer, and external queues before concrete Box destruction;
7. clear the pool and verify state/object/root/string/estimated-byte/work-queue
   counts are zero before transitioning to `Closed`.

Owner-thread `Drop` calls the same close implementation. Successful explicit
close is idempotent and returns the first teardown totals with
`already_closed = true` on later calls. A theoretically impossible
foreign-thread/busy Drop reached only through unsafe host invariant violation
uses a no-callback leak-protection fallback; explicit foreign or busy close is
rejected before mutation.

`RuntimeCloseReport` exposes rejected open-Upvalue edges, checked owner-handle
mismatches, missing active stack values, pending finalizer entries discarded,
and two unconditional capability debts:

- close-time Lua-visible `__gc` callbacks are not run;
- library-specific IO/resource drain hooks are not run.

Rust typed Userdata payload destructors still run exactly once during concrete
Box destruction. Their backing storage is now explicitly 16-byte aligned;
over-aligned types are rejected, safe raw-byte views are unavailable while a
typed value is live, and successful destruction zeroes the storage before raw
views are restored. This fixes the previous `Vec<u8>` alignment and typed/raw
aliasing contract, including the `IoFileData` payload.

Tests cover all seven concrete GC dispatch layouts, non-fixed Thread-first and
fixed-last order, ordinary/fixed byte DropProbes, stale Upvalue rejection,
main/coroutine Upvalue closure, idempotence, Drop delegation, zeroed reports,
and 1,000 Runtime/coroutine close cycles. This path never calls `collect`,
`sweep`, `clear_all`, weak reconciliation, or Lua callback dispatch. Live-VM
destructive collection remains gated on phase G and the rest of section 10.

### F. Internal stop-the-world collection — M1.9

- implement a Runtime-only full cycle and state pre-sweep;
- replace fake object/memory counters with actual accounting;
- keep the Lua-visible destructive route gated while G is incomplete.

Acceptance: unreachable probes are destroyed and reachable probes survive;
object and byte counts decrease without stale-handle access.

### G. Weak tables, finalizers, and public full collection — M1.9/M1.12

- use the order mark, prepare finalizers, propagate resurrected graphs,
  reconcile/clear weak tables, pre-close unreachable states, sweep, run
  finalizers;
- keep pending finalizers rooted across reentrancy;
- isolate callback errors, restore stacks, enforce exactly-once finalization,
  resurrection, and close drain;
- expose real `collectgarbage("collect")` only after this phase passes.

Weak-key/value behavior follows the pinned C++/Lua 5.1 oracle; Lua 5.2+
ephemeron assumptions must not be imported without a differential test.

### H. Mutation API, barriers, and accounting — M1.11

- make edge-bearing fields private;
- require `MutationContext` for every object/root write;
- validate both owner and child heap identities;
- publish new allocation graphs during active cycles;
- update accounted size and debt after container capacity changes.

Acceptance: directed black-to-white tests for every mutation family, root
changes, new allocations, weak-mode changes, and cross-heap rejection.

### I. Incremental collector — M1.10

- implement real phases, budgeted work, debt threshold, pause, multiplier,
  stop/restart, and cycle-completion reporting;
- rescan wide roots at initial and atomic boundaries;
- test mutation and allocation in every phase.

Incremental collection remains disabled until H is complete.

### J. Durability and unsafe validation — M1.13

- add 1,000-cycle runtime/state/coroutine/weak/finalizer/upvalue/dump tests;
- require zero live state/object/accounted/allocator/pending counts at close;
- run targeted Miri and Linux ASan jobs over the unsafe lifecycle paths;
- run Lua 5.1 and pinned C++ GC/lifecycle differential fixtures.

## 9. Unsafe invariants

Every `unsafe` block participating in runtime ownership must cite one of these
invariants in its safety comment. A new invariant requires an RFC and inventory
update.

| ID | Invariant | Required enforcement |
|---|---|---|
| U-01 | A managed object's header is at the layout position expected by collector dispatch. | Sealed managed-object construction plus layout/type-tag tests. |
| U-02 | A GC handle is dereferenced only after heap id, generation, type, and liveness validation. | P1 checked collector entry points use pointer + process-global ObjectId + tag; final enforcement still requires Heap-owned scoped lookup and removal of transitional direct `GcRef::as_ref` call sites. |
| U-03 | A scoped object reference cannot coexist with destructive collection or a second mutable reference. | `VmContext` borrow and no stored raw collector alias. |
| U-04 | A `StateHandle` names one occupied slot in its originating runtime and generation. | Non-duplicable namespace issuer, opaque identities, checked arena lookup, checked generation advance, MAX-generation retirement, and close-time arena invariant preflight. |
| U-05 | An open Upvalue's state and stack slot remain valid until it is closed. | State-handle ownership; close before state removal; bounds checks. |
| U-06 | Active Function/Proto metadata is a validated GC edge, never an unrooted raw pointer. | Handle-bearing `CallInfo`; trace active frame owner. |
| U-07 | An FFI `LuaState` pointer is valid only for the dynamic callback extent. | Scoped reconstruction inside each callback; no `'static` helper. |
| U-08 | Mutable runtime access occurs on the owner thread; only the cancellation handle crosses threads. | `!Send`/`!Sync`, owner check, foreign-thread tests. |
| U-09 | Every collector queue pointer/handle is live and is removed before object destruction. | Central `destroy_object`; pending/weak/gray/root queue assertions. |
| U-10 | A dump/cache entry never owns an unrooted or foreign-heap GC handle. | Owned bytes or runtime RootToken; static/TLS scan. |
| U-11 | States/upvalues and services are closed while the GC objects they need are still alive. | Explicit close phase and thread-first heap teardown. |
| U-12 | Every published edge preserves heap provenance, tri-color, and memory-accounting invariants. | Private mutation fields and mandatory `MutationContext`. |

## 10. Hard gate before destructive sweep

All boxes are required before a sweep can run against a live VM:

- [ ] one unique `Runtime`/`Heap` owner and verified dependency lifetime;
- [ ] no standalone collector drop path that lacks a live string pool and
      allocator;
- [ ] main and coroutine states have a single, reclaiming owner;
- [ ] no `Box::into_raw(LuaState)` ownership transfer and no runtime
      `&'static mut` helper;
- [x] the thread-local dump Proto registry is removed and pseudo-dump loading is rejected;
- [ ] every handle is provenance/liveness checked and cross-heap publication is
      rejected;
- [ ] the complete root inventory is traced through one implementation;
- [x] active Function/Proto and open-Upvalue owners are handles, not lifetime
      placeholders;
- [ ] temporary allocation/publication roots are enforced by API shape;
- [ ] pending finalizers are roots and all work queues are pruned on destroy;
- [ ] unreachable coroutine states close before Thread/object sweep;
- [x] ordinary and fixed objects have a deterministic shutdown destruction route;
- [x] mark-only root tests, shutdown Drop probes, and inventory validation pass.

Complete write barriers are not required for a strictly non-reentrant,
stop-the-world *internal* collector prototype. They are an absolute prerequisite
for incremental collection. Complete weak/finalizer behavior is required
before destructive full collection becomes Lua-visible.

## 11. Acceptance commands

The documentation and inventory gate is available immediately:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass `
  -File tools/check_gc_root_inventory.ps1
```

The existing repository gates remain mandatory:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass `
  -File tools/m0_compatibility_gate.ps1 `
  -CppRoot ..\lua_cpp

powershell -NoProfile -ExecutionPolicy Bypass `
  -File tools/rust_quality_gate.ps1
```

The following commands become required as their named test targets are added
by phases A-J:

```powershell
cargo test -p lua_vm --test runtime_ownership
cargo test -p lua_vm --test runtime_gc_roots
cargo test -p lua_core --test gc_barriers
cargo test -p lua_stdlib --test gc_semantics
cargo test -p lua_app --test runtime_durability --release
```

Targeted unsafe checks should run in a Linux CI lane:

```bash
cargo +nightly miri test -p lua_core --lib gc::
cargo +nightly miri test -p lua_vm --test runtime_ownership
RUSTFLAGS="-Zsanitizer=address" \
  cargo +nightly test -Zbuild-std \
  --target x86_64-unknown-linux-gnu --workspace --tests
```

A future `tools/m1_memory_gate.ps1` should aggregate the A-J tests, run the
1,000-cycle workload, and emit a machine-readable final assertion:

```text
live_states == 0
gc_objects == 0
gc_accounted_bytes == 0
allocator_live_bytes == 0
pending_finalizers == 0
```
