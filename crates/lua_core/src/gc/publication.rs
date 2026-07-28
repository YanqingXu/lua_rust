//! Lexically scoped roots for objects that are not yet published into a
//! collector-visible object graph.
//!
//! Allocation-triggered collection is intentionally still disabled. This
//! module establishes the root registry and branded transaction API required
//! before such collection can be enabled. Safe code cannot extract a raw
//! `GcRef` from [`Rooted`]; the only initial publication operation promotes an
//! object into the collector's explicit root set.

use std::fmt;
use std::marker::PhantomData;

use crate::function::{Function, RuntimeNativeFunction};
use crate::gc::collector::{GarbageCollector, GcRefValidationError};
use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
#[cfg(test)]
use crate::gc::mark::MarkRootSeedReport;
use crate::gc::object_id::ObjectId;
use crate::thread::Thread;
use crate::upvalue::Upvalue;
use crate::value::Value;

/// A collector-managed object protected for one publication transaction.
///
/// The lifetime is branded by `GarbageCollector::with_publication`. Fields are
/// private and there is deliberately no method returning `GcRef<T>`: graph
/// publication must use a checked operation on [`PublicationTxn`].
pub struct Rooted<'scope, T: GcObject> {
    reference: GcRef<T>,
    temporary_root_id: u64,
    _scope: PhantomData<&'scope mut ()>,
}

impl<T: GcObject> Rooted<'_, T> {
    /// Process-unique identity of the protected allocation.
    pub fn object_id(&self) -> ObjectId {
        self.reference.object_id()
    }
}

impl<T: GcObject> fmt::Debug for Rooted<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Rooted")
            .field("reference", &self.reference)
            .field("temporary_root_id", &self.temporary_root_id)
            .finish_non_exhaustive()
    }
}

/// One lexical object-publication transaction.
///
/// Every allocation made through this type is inserted into the collector's
/// temporary root registry before its branded handle is returned. Dropping
/// the transaction, including during panic unwinding, removes exactly the
/// roots registered by this transaction.
pub struct PublicationTxn<'scope> {
    collector: &'scope mut GarbageCollector,
    temporary_root_ids: Vec<u64>,
}

impl<'scope> PublicationTxn<'scope> {
    fn new(collector: &'scope mut GarbageCollector) -> Self {
        Self {
            collector,
            temporary_root_ids: Vec::new(),
        }
    }

    /// Allocate an object and protect it before exposing its branded handle.
    ///
    /// `GarbageCollector::create` does not trigger collection. Registry and
    /// transaction capacity are reserved before allocation so that the
    /// infallible identity insertion is the only step between registration
    /// and protection.
    pub fn alloc<T: GcObject>(&mut self, object: T) -> Rooted<'scope, T> {
        self.prepare_registration();
        let temporary_root_id = self.collector.allocate_temporary_root_id();
        let reference = self.collector.create(object);

        // Capacity was reserved before object allocation. Record ownership in
        // the transaction first, so an unexpected panic during map insertion
        // still lets Drop attempt the exact-id cleanup.
        self.temporary_root_ids.push(temporary_root_id);
        let replaced = self
            .collector
            .temporary_roots
            .insert(temporary_root_id, reference.erase());
        assert!(
            replaced.is_none(),
            "temporary publication root identity was unexpectedly reused"
        );

        Rooted {
            reference,
            temporary_root_id,
            _scope: PhantomData,
        }
    }

    /// Protect an already registered allocation for this transaction.
    pub fn protect<T: GcObject>(
        &mut self,
        reference: GcRef<T>,
    ) -> Result<Rooted<'scope, T>, GcRefValidationError> {
        self.collector.validate_ref(reference)?;
        self.prepare_registration();
        let temporary_root_id = self.collector.allocate_temporary_root_id();
        self.temporary_root_ids.push(temporary_root_id);
        let replaced = self
            .collector
            .temporary_roots
            .insert(temporary_root_id, reference.erase());
        assert!(
            replaced.is_none(),
            "temporary publication root identity was unexpectedly reused"
        );

        Ok(Rooted {
            reference,
            temporary_root_id,
            _scope: PhantomData,
        })
    }

    /// Read a protected allocation through the collector's checked borrow.
    pub fn with_ref<T: GcObject, R>(
        &self,
        rooted: &Rooted<'scope, T>,
        read: impl for<'a> FnOnce(&'a T) -> R,
    ) -> Result<R, GcRefValidationError> {
        self.validate_protection(rooted)?;
        self.collector.with_ref(rooted.reference, read)
    }

    /// Mutate a protected allocation through the collector's checked borrow.
    pub fn with_mut<T: GcObject, R>(
        &mut self,
        rooted: &Rooted<'scope, T>,
        write: impl for<'a> FnOnce(&'a mut T) -> R,
    ) -> Result<R, GcRefValidationError> {
        self.validate_protection(rooted)?;
        self.collector.with_mut(rooted.reference, write)
    }

    /// Safely publish a protected object as an explicit collector root.
    ///
    /// The explicit root is installed before the temporary root is released.
    /// This is intentionally the only initial operation that returns a raw
    /// `GcRef`; object-graph publication will be added as typed methods rather
    /// than an unchecked generic `commit`.
    pub fn publish_as_explicit_root<T: GcObject>(
        &mut self,
        rooted: Rooted<'scope, T>,
    ) -> Result<GcRef<T>, GcRefValidationError> {
        self.validate_protection(&rooted)?;
        self.collector.add_root(rooted.reference);
        self.release_owned_root(rooted.temporary_root_id);
        Ok(rooted.reference)
    }

    /// Build a protected closed Upvalue that owns a protected Thread value.
    ///
    /// Both nodes remain temporary roots until their transaction-owned
    /// handles are either published or the transaction ends.
    pub fn alloc_closed_thread_upvalue(
        &mut self,
        thread: &Rooted<'scope, Thread>,
    ) -> Result<Rooted<'scope, Upvalue>, GcRefValidationError> {
        self.validate_protection(thread)?;
        Ok(self.alloc(Upvalue::new_closed(Value::Thread(thread.reference))))
    }

    /// Build a protected Runtime-native closure around a protected Upvalue.
    pub fn alloc_runtime_native_with_upvalue(
        &mut self,
        operation: RuntimeNativeFunction,
        upvalue: &Rooted<'scope, Upvalue>,
    ) -> Result<Rooted<'scope, Function>, GcRefValidationError> {
        self.validate_protection(upvalue)?;
        let mut function = Function::new_runtime_native(operation);
        function.add_upvalue(upvalue.reference);
        Ok(self.alloc(function))
    }

    /// Check that a protected Function retains a protected Thread through one
    /// of its closed Upvalues.
    pub fn function_reaches_thread(
        &self,
        function: &Rooted<'scope, Function>,
        thread: &Rooted<'scope, Thread>,
    ) -> Result<bool, GcRefValidationError> {
        self.validate_protection(function)?;
        self.validate_protection(thread)?;
        self.function_reaches_thread_object_id(function, thread.object_id())
    }

    /// Check that a protected Function retains the live Thread with an exact
    /// object identity through one of its closed Upvalues.
    ///
    /// This variant supports the handoff from an object temporary root to a
    /// PendingState root without re-exposing the Thread's raw `GcRef`.
    pub fn function_reaches_thread_object_id(
        &self,
        function: &Rooted<'scope, Function>,
        thread_id: ObjectId,
    ) -> Result<bool, GcRefValidationError> {
        self.validate_protection(function)?;
        let upvalues = self.collector.with_ref(function.reference, |function| {
            (0..function.upvalue_count())
                .filter_map(|index| function.upvalue(index))
                .collect::<Vec<_>>()
        })?;
        for upvalue in upvalues {
            let reaches = self.collector.with_ref(upvalue, |upvalue| {
                upvalue.is_closed()
                    && matches!(
                        upvalue.get_closed_value(),
                        Value::Thread(candidate) if candidate.object_id() == thread_id
                    )
            })?;
            if reaches {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Publish a protected Thread value while its temporary root remains
    /// installed for the entire callback.
    ///
    /// If the callback unwinds, transaction Drop retains responsibility for
    /// removing the temporary root. On normal return the callback must have
    /// installed the supplied value into a traced owner before this method
    /// releases the temporary identity.
    ///
    /// # Safety
    ///
    /// On normal return, `publish` must either install the supplied Value in
    /// an independently traced owner or consume it synchronously. It must not
    /// let the embedded `GcRef` escape only through an unrooted local or
    /// return value.
    pub unsafe fn publish_thread_value<R>(
        &mut self,
        rooted: Rooted<'scope, Thread>,
        publish: impl FnOnce(Value) -> R,
    ) -> Result<R, GcRefValidationError> {
        self.validate_protection(&rooted)?;
        let result = publish(Value::Thread(rooted.reference));
        self.release_owned_root(rooted.temporary_root_id);
        Ok(result)
    }

    /// Publish a protected Function value while its temporary root remains
    /// installed for the entire callback.
    ///
    /// # Safety
    ///
    /// On normal return, `publish` must either install the supplied Value in
    /// an independently traced owner or consume it synchronously. It must not
    /// let the embedded `GcRef` escape only through an unrooted local or
    /// return value.
    pub unsafe fn publish_function_value<R>(
        &mut self,
        rooted: Rooted<'scope, Function>,
        publish: impl FnOnce(Value) -> R,
    ) -> Result<R, GcRefValidationError> {
        self.validate_protection(&rooted)?;
        let result = publish(Value::Function(rooted.reference));
        self.release_owned_root(rooted.temporary_root_id);
        Ok(result)
    }

    /// Run a nested publication transaction while retaining all outer roots.
    pub fn with_nested<R>(
        &mut self,
        publish: impl for<'nested> FnOnce(&mut PublicationTxn<'nested>) -> R,
    ) -> R {
        let mut nested = PublicationTxn::new(&mut *self.collector);
        publish(&mut nested)
    }

    #[cfg(test)]
    fn trace_mark_only(&mut self) -> MarkRootSeedReport {
        self.collector.begin_mark_only()
    }

    /// Number of temporary roots currently registered in this collector,
    /// including roots owned by enclosing publication transactions.
    pub fn active_temporary_root_count(&self) -> usize {
        self.collector.temporary_root_count()
    }

    fn prepare_registration(&mut self) {
        self.temporary_root_ids.reserve(1);
        self.collector.temporary_roots.reserve(1);
    }

    fn validate_protection<T: GcObject>(
        &self,
        rooted: &Rooted<'scope, T>,
    ) -> Result<(), GcRefValidationError> {
        self.collector.validate_ref(rooted.reference)?;
        let expected = rooted.reference.erase();
        match self
            .collector
            .temporary_roots
            .get(&rooted.temporary_root_id)
        {
            Some(actual) if *actual == expected => Ok(()),
            _ => Err(GcRefValidationError::NotLive {
                object_id: rooted.reference.object_id(),
            }),
        }
    }

    fn release_owned_root(&mut self, temporary_root_id: u64) {
        self.temporary_root_ids
            .retain(|candidate| *candidate != temporary_root_id);
        if self
            .collector
            .temporary_roots
            .remove(&temporary_root_id)
            .is_none()
        {
            self.collector.rejected_temporary_root_releases = self
                .collector
                .rejected_temporary_root_releases
                .saturating_add(1);
        }
    }
}

impl Drop for PublicationTxn<'_> {
    fn drop(&mut self) {
        for temporary_root_id in self.temporary_root_ids.drain(..) {
            if self
                .collector
                .temporary_roots
                .remove(&temporary_root_id)
                .is_none()
            {
                self.collector.rejected_temporary_root_releases = self
                    .collector
                    .rejected_temporary_root_releases
                    .saturating_add(1);
            }
        }
    }
}

impl GarbageCollector {
    /// Execute one branded lexical publication scope.
    ///
    /// The higher-ranked closure prevents `Rooted<'scope, T>` from appearing
    /// in the return type. A raw handle can leave the scope only through a
    /// checked publication method such as
    /// `PublicationTxn::publish_as_explicit_root`.
    ///
    /// ```compile_fail
    /// use lua_core::gc::collector::GarbageCollector;
    /// use lua_core::table::Table;
    ///
    /// let mut collector = GarbageCollector::new();
    /// let _escaped = collector.with_publication(|transaction| {
    ///     transaction.alloc(Table::new())
    /// });
    /// ```
    pub fn with_publication<R>(
        &mut self,
        publish: impl for<'scope> FnOnce(&mut PublicationTxn<'scope>) -> R,
    ) -> R {
        let mut transaction = PublicationTxn::new(self);
        publish(&mut transaction)
    }

    /// Number of active lexical object-publication roots.
    pub fn temporary_root_count(&self) -> usize {
        self.temporary_roots.len()
    }

    /// Number of exact-id temporary-root releases rejected by the registry.
    pub fn rejected_temporary_root_release_count(&self) -> usize {
        self.rejected_temporary_root_releases
    }

    fn allocate_temporary_root_id(&mut self) -> u64 {
        let identity = self.next_temporary_root_id;
        self.next_temporary_root_id = identity
            .checked_add(1)
            .expect("temporary publication root identity space exhausted");
        identity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc_string::GcString;
    use crate::string_pool::StringPool;
    use crate::table::Table;

    #[test]
    fn allocation_is_seeded_until_transaction_drop() {
        let mut collector = GarbageCollector::new();

        collector.with_publication(|transaction| {
            let rooted = transaction.alloc(Table::new());
            assert_eq!(transaction.active_temporary_root_count(), 1);
            let report = transaction.trace_mark_only();
            assert_eq!(report.temporary_seeded, 1);
            assert_eq!(report.temporary_rejected, 0);
            assert!(transaction.with_ref(&rooted, |_| ()).is_ok());
        });

        assert_eq!(collector.temporary_root_count(), 0);
        assert_eq!(collector.rejected_temporary_root_release_count(), 0);
        let report = collector.begin_mark_only();
        assert_eq!(report.temporary_seeded, 0);
    }

    #[test]
    fn nested_and_panic_scopes_release_exact_roots() {
        let mut collector = GarbageCollector::new();
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            collector.with_publication(|outer| {
                let _outer = outer.alloc(Table::new());
                assert_eq!(outer.active_temporary_root_count(), 1);
                outer.with_nested(|inner| {
                    let _inner = inner.alloc(GcString::from_bytes(b"nested"));
                    assert_eq!(inner.active_temporary_root_count(), 2);
                    let report = inner.trace_mark_only();
                    assert_eq!(report.temporary_seeded, 2);
                    panic!("injected publication failure");
                });
            });
        }));

        assert!(unwind.is_err());
        assert_eq!(collector.temporary_root_count(), 0);
        assert_eq!(collector.rejected_temporary_root_release_count(), 0);
    }

    #[test]
    fn explicit_root_promotion_never_leaves_an_unrooted_gap() {
        let mut collector = GarbageCollector::new();
        let published = collector.with_publication(|transaction| {
            let rooted = transaction.alloc(Table::new());
            transaction
                .publish_as_explicit_root(rooted)
                .expect("new allocation remains registered")
        });

        assert_eq!(collector.temporary_root_count(), 0);
        assert!(collector.is_root(published));
        assert!(collector.contains_registered(published));
    }

    #[test]
    fn typed_coroutine_wrapper_graph_stays_rooted_through_publication() {
        let mut collector = GarbageCollector::new();
        let mut published_function_seen = false;

        collector.with_publication(|transaction| {
            let thread = transaction.alloc(Thread::new());
            let upvalue = transaction
                .alloc_closed_thread_upvalue(&thread)
                .expect("protected Thread builds a protected Upvalue");
            let wrapper = transaction
                .alloc_runtime_native_with_upvalue(
                    RuntimeNativeFunction::CoroutineWrapRunner,
                    &upvalue,
                )
                .expect("protected Upvalue builds a protected wrapper");

            assert_eq!(transaction.active_temporary_root_count(), 3);
            assert!(
                transaction
                    .function_reaches_thread(&wrapper, &thread)
                    .expect("the protected wrapper graph validates")
            );
            // SAFETY: the callback inspects the Value synchronously and does
            // not let its embedded GcRef escape.
            unsafe {
                transaction.publish_function_value(wrapper, |value| {
                    assert_eq!(transaction_root_kind(&value), "function");
                    published_function_seen = true;
                })
            }
            .expect("wrapper publication remains checked");
            assert_eq!(transaction.active_temporary_root_count(), 2);
        });

        assert!(published_function_seen);
        assert_eq!(collector.temporary_root_count(), 0);
        assert_eq!(collector.rejected_temporary_root_release_count(), 0);
    }

    #[test]
    fn typed_value_publication_panic_releases_exact_root() {
        let mut collector = GarbageCollector::new();
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            collector.with_publication(|transaction| {
                let thread = transaction.alloc(Thread::new());
                // SAFETY: the callback never returns normally, so transaction
                // Drop retains and then removes the exact root.
                unsafe {
                    transaction.publish_thread_value(thread, |_| {
                        panic!("injected stack publication failure");
                    })
                }
                .expect("panic occurs inside the publication callback");
            });
        }));

        assert!(unwind.is_err());
        assert_eq!(collector.temporary_root_count(), 0);
        assert_eq!(collector.rejected_temporary_root_release_count(), 0);
    }

    fn transaction_root_kind(value: &Value) -> &'static str {
        match value {
            Value::Function(_) => "function",
            _ => "other",
        }
    }

    #[test]
    fn protect_rejects_foreign_and_stale_handles_before_registration() {
        let mut owner = GarbageCollector::new();
        let mut foreign = GarbageCollector::new();
        let mut owner_pool = StringPool::new();
        let live = owner.create(Table::new());
        let stale = owner.create(Table::new());
        owner.add_root(live);
        owner.collect(&mut owner_pool);

        foreign.with_publication(|transaction| {
            assert!(transaction.protect(live).is_err());
            assert!(transaction.protect(stale).is_err());
            assert_eq!(transaction.active_temporary_root_count(), 0);
        });
        owner.with_publication(|transaction| {
            assert!(transaction.protect(stale).is_err());
            let _protected = transaction
                .protect(live)
                .expect("the owning collector accepts the exact live identity");
            assert_eq!(transaction.active_temporary_root_count(), 1);
        });
    }

    #[test]
    fn one_thousand_scopes_return_registry_to_zero() {
        let mut collector = GarbageCollector::new();

        for _ in 0..1_000 {
            collector.with_publication(|transaction| {
                let rooted = transaction.alloc(Table::new());
                transaction
                    .with_mut(&rooted, |table| table.set_flags(7))
                    .expect("temporary root remains registered");
                assert_eq!(transaction.active_temporary_root_count(), 1);
            });
            assert_eq!(collector.temporary_root_count(), 0);
        }

        assert_eq!(collector.rejected_temporary_root_release_count(), 0);
    }
}
