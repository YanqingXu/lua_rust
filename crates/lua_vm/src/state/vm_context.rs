//! Dynamically scoped access to the unique Heap services for one state turn.
//!
//! `LuaState` deliberately stores no collector or string-pool pointer. Runtime
//! entry points install a context only for the dynamic extent in which the
//! state is exclusively active. Native callbacks recover the services through
//! the current-state check below.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use lua_core::gc::collector::GarbageCollector;
use lua_core::string_pool::StringPool;

use super::LuaState;

static NEXT_SCOPE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
struct ActiveServices {
    scope_id: u64,
    state: NonNull<LuaState>,
    collector: NonNull<GarbageCollector>,
    strings: NonNull<StringPool>,
}

thread_local! {
    static ACTIVE_SERVICES: RefCell<Vec<ActiveServices>> = const { RefCell::new(Vec::new()) };
}

/// RAII token for one dynamically scoped VM service borrow.
pub(crate) struct ActiveVmContext {
    scope_id: u64,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl Drop for ActiveVmContext {
    fn drop(&mut self) {
        ACTIVE_SERVICES.with(|active| {
            let popped = active.borrow_mut().pop();
            assert_eq!(
                popped.map(|entry| entry.scope_id),
                Some(self.scope_id),
                "VM service scopes must unwind in stack order"
            );
        });
    }
}

/// Install raw service pointers whose lifetime is managed by the caller.
///
/// # Safety
///
/// All three pointers must remain live, exclusively accessible, and on the
/// current thread until the returned token is dropped. The token must be
/// dropped before any pointed-to state or service can be moved or destroyed.
pub(crate) unsafe fn enter_vm_context(
    state: NonNull<LuaState>,
    mut collector: NonNull<GarbageCollector>,
    mut strings: NonNull<StringPool>,
) -> ActiveVmContext {
    // SAFETY: caller guarantees exclusive live access for the scope.
    let collector_ref = unsafe { collector.as_mut() };
    // SAFETY: same caller guarantee; the pool is disjoint from the collector.
    let strings_ref = unsafe { strings.as_mut() };
    strings_ref.bind_or_assert_owner(collector_ref.heap_id());
    assert_eq!(
        strings_ref.heap_id(),
        Some(collector_ref.heap_id()),
        "VM context requires services from one Heap"
    );
    let scope_id = NEXT_SCOPE_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("VM service-scope identity space exhausted");
    let entry = ActiveServices {
        scope_id,
        state,
        collector,
        strings,
    };
    ACTIVE_SERVICES.with(|active| active.borrow_mut().push(entry));
    ActiveVmContext {
        scope_id,
        _not_send_or_sync: PhantomData,
    }
}

pub(crate) fn active_service_pointers(
    state: &LuaState,
) -> Option<(NonNull<GarbageCollector>, NonNull<StringPool>)> {
    let state = NonNull::from(state);
    ACTIVE_SERVICES.with(|active| {
        active
            .borrow()
            .last()
            .filter(|entry| entry.state == state)
            .map(|entry| (entry.collector, entry.strings))
    })
}

/// Run code with one state and both services from the same Heap active.
///
/// This is primarily for host integration and tests that must call lower-level
/// VM helpers while preserving the same dynamic context as Runtime execution.
pub fn with_vm_context_parts<R>(
    state: &mut LuaState,
    collector: &mut GarbageCollector,
    strings: &mut StringPool,
    execute: impl FnOnce(&mut LuaState, &mut GarbageCollector, &mut StringPool) -> R,
) -> R {
    // SAFETY: the closure cannot outlive these exclusive references and the
    // token is dropped before this function returns or unwinds.
    let _scope = unsafe {
        enter_vm_context(
            NonNull::from(&mut *state),
            NonNull::from(&mut *collector),
            NonNull::from(&mut *strings),
        )
    };
    execute(state, collector, strings)
}

/// Run host/test code with one state and one Heap service pair active.
///
/// Production Runtime entry points install the same scope automatically.
pub fn with_vm_context<R>(
    state: &mut LuaState,
    collector: &mut GarbageCollector,
    strings: &mut StringPool,
    execute: impl FnOnce(&mut LuaState) -> R,
) -> R {
    // SAFETY: the closure cannot outlive these exclusive references and the
    // token is dropped before this function returns or unwinds.
    let _scope = unsafe {
        enter_vm_context(
            NonNull::from(&mut *state),
            NonNull::from(&mut *collector),
            NonNull::from(&mut *strings),
        )
    };
    execute(state)
}

#[cfg(test)]
mod tests {
    use lua_core::heap::Heap;

    use super::*;

    #[test]
    fn context_is_visible_only_to_the_active_state() {
        let mut heap = Heap::new();
        let mut active = LuaState::new();
        let inactive = LuaState::new();

        heap.with_parts_mut(|collector, strings| {
            assert!(active.active_gc_ptr().is_none());
            with_vm_context(&mut active, collector, strings, |active| {
                assert!(active.active_gc_ptr().is_some());
                assert!(active.active_string_pool_ptr().is_some());
                assert!(inactive.active_gc_ptr().is_none());
            });
            assert!(active.active_gc_ptr().is_none());
        });
    }

    #[test]
    fn nested_context_restores_the_previous_state() {
        let mut heap = Heap::new();
        let mut first = LuaState::new();
        let mut second = LuaState::new();

        heap.with_parts_mut(|collector, strings| {
            // SAFETY: all values outlive both lexical tokens below.
            let _outer = unsafe {
                enter_vm_context(
                    NonNull::from(&mut first),
                    NonNull::from(&mut *collector),
                    NonNull::from(&mut *strings),
                )
            };
            assert!(first.active_gc_ptr().is_some());
            {
                // SAFETY: all values outlive this nested lexical token.
                let _inner = unsafe {
                    enter_vm_context(
                        NonNull::from(&mut second),
                        NonNull::from(&mut *collector),
                        NonNull::from(&mut *strings),
                    )
                };
                assert!(first.active_gc_ptr().is_none());
                assert!(second.active_gc_ptr().is_some());
            }
            assert!(first.active_gc_ptr().is_some());
        });
    }

    #[test]
    fn panic_unwinds_the_dynamic_service_scope() {
        let mut heap = Heap::new();
        let mut state = LuaState::new();

        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            heap.with_parts_mut(|collector, strings| {
                with_vm_context(&mut state, collector, strings, |_| {
                    panic!("injected VM-context failure");
                });
            });
        }));

        assert!(unwind.is_err());
        assert!(state.active_gc_ptr().is_none());
        assert!(state.active_string_pool_ptr().is_none());
    }

    #[test]
    fn context_rejects_services_from_different_heaps() {
        let mut collector_heap = Heap::new();
        let mut strings_heap = Heap::new();
        let mut state = LuaState::new();

        let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let collector = collector_heap.collector_mut();
            let strings = strings_heap.strings_mut();
            with_vm_context(&mut state, collector, strings, |_| ());
        }));

        assert!(rejected.is_err());
        assert!(state.active_gc_ptr().is_none());
    }
}
