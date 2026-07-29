//! Lexically scoped roots for objects that are not yet published into a
//! collector-visible object graph.
//!
//! Allocation-triggered collection is intentionally still disabled. This
//! module establishes the root registry and branded transaction API required
//! before such collection can be enabled. Safe code cannot extract a raw
//! `GcRef` from [`Rooted`]. Typed operations validate and publish supported
//! object graphs, while explicit-root promotion remains available for
//! top-level owners.

use std::fmt;
use std::marker::PhantomData;

use crate::function::{CFunction, Function, RuntimeNativeFunction};
use crate::gc::collector::{GarbageCollector, GcRefValidationError};
use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc::mark::MarkRootSeedReport;
use crate::gc::object_id::ObjectId;
use crate::gc_string::GcString;
use crate::proto::Proto;
use crate::string_pool::StringPool;
use crate::table::Table;
use crate::thread::Thread;
use crate::upvalue::Upvalue;
use crate::userdata::Userdata;
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
    /// `GcRef`; object-graph publication uses typed methods rather than an
    /// unchecked generic `commit`.
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

    /// Allocate a protected C Function.
    pub fn alloc_c_function(&mut self, function: CFunction) -> Rooted<'scope, Function> {
        self.alloc(Function::new_c(function))
    }

    /// Allocate a protected Runtime-native Function without Upvalues.
    pub fn alloc_runtime_native_function(
        &mut self,
        operation: RuntimeNativeFunction,
    ) -> Rooted<'scope, Function> {
        self.alloc(Function::new_runtime_native(operation))
    }

    /// Intern one Lua byte string and retain it as a publication root.
    pub fn intern_bytes(
        &mut self,
        pool: &mut StringPool,
        bytes: &[u8],
    ) -> Result<Rooted<'scope, GcString>, GcRefValidationError> {
        pool.bind_or_assert_owner(self.collector.heap_id());
        if let Some(existing) = pool.find_bytes(bytes) {
            if self.collector.contains_registered(existing) {
                return self.protect(existing);
            }
            pool.remove(existing);
        }

        pool.reserve(1);
        let rooted = self.alloc(GcString::from_bytes(bytes));
        pool.insert_reserved_bytes(bytes, rooted.reference);
        Ok(rooted)
    }

    /// Retain a protected string for attachment to an unpublished Proto.
    ///
    /// # Safety
    ///
    /// The returned handle must be attached only to a Proto graph that is
    /// itself protected or published before this transaction ends. It must
    /// not escape as an independently unrooted handle.
    pub unsafe fn retain_string_for_proto(
        &self,
        rooted: Rooted<'scope, GcString>,
    ) -> Result<GcRef<GcString>, GcRefValidationError> {
        self.validate_protection(&rooted)?;
        Ok(rooted.reference)
    }

    /// Retain a protected child Proto for attachment to an unpublished parent.
    ///
    /// # Safety
    ///
    /// The returned handle must be attached only to a Proto graph that is
    /// itself protected or published before this transaction ends. It must
    /// not escape as an independently unrooted handle.
    pub unsafe fn retain_proto_for_parent(
        &self,
        rooted: Rooted<'scope, Proto>,
    ) -> Result<GcRef<Proto>, GcRefValidationError> {
        self.validate_protection(&rooted)?;
        Ok(rooted.reference)
    }

    /// Allocate a protected Lua Function that retains a protected Proto.
    pub fn alloc_lua_function(
        &mut self,
        proto: &Rooted<'scope, Proto>,
    ) -> Result<Rooted<'scope, Function>, GcRefValidationError> {
        self.validate_protection(proto)?;
        Ok(self.alloc(Function::new_lua(proto.reference)))
    }

    /// Build a protected Lua closure from collector-validated live edges.
    ///
    /// This is the VM-facing counterpart to [`Self::alloc_lua_function`]:
    /// nested Protos, environments, and open Upvalues are already reachable
    /// from the active execution graph rather than newly allocated in this
    /// transaction.
    pub fn alloc_lua_closure(
        &mut self,
        proto: GcRef<Proto>,
        environment: Option<GcRef<Table>>,
        upvalues: &[GcRef<Upvalue>],
    ) -> Result<Rooted<'scope, Function>, GcRefValidationError> {
        self.collector.validate_ref(proto)?;
        if let Some(environment) = environment {
            self.collector.validate_ref(environment)?;
        }
        for upvalue in upvalues {
            self.collector.validate_ref(*upvalue)?;
        }

        let mut function = Function::new_lua(proto);
        function.set_env(environment);
        for upvalue in upvalues {
            function.add_upvalue(*upvalue);
        }
        Ok(self.alloc(function))
    }

    /// Install a validated environment on a protected Function.
    pub fn set_function_environment(
        &mut self,
        function: &Rooted<'scope, Function>,
        environment: Option<GcRef<Table>>,
    ) -> Result<(), GcRefValidationError> {
        self.validate_protection(function)?;
        if let Some(environment) = environment {
            self.collector.validate_ref(environment)?;
        }
        self.collector
            .with_mut(function.reference, |function| function.set_env(environment))
    }

    /// Install a protected Table as the environment of a protected Function.
    pub fn set_function_rooted_environment(
        &mut self,
        function: &Rooted<'scope, Function>,
        environment: Option<&Rooted<'scope, Table>>,
    ) -> Result<(), GcRefValidationError> {
        self.validate_protection(function)?;
        let environment = match environment {
            Some(environment) => {
                self.validate_protection(environment)?;
                Some(environment.reference)
            }
            None => None,
        };
        self.collector
            .with_mut(function.reference, |function| function.set_env(environment))
    }

    /// Install a validated environment on a protected Lua Function.
    pub fn set_lua_function_environment(
        &mut self,
        function: &Rooted<'scope, Function>,
        environment: Option<GcRef<Table>>,
    ) -> Result<(), GcRefValidationError> {
        self.set_function_environment(function, environment)
    }

    /// Attach an already-live Value under a protected string key in a
    /// protected Table.
    ///
    /// Collectable values are validated against this transaction's collector
    /// before the edge is installed.
    pub fn set_table_value(
        &mut self,
        table: &Rooted<'scope, Table>,
        key: &Rooted<'scope, GcString>,
        value: &Value,
    ) -> Result<(), GcRefValidationError> {
        self.validate_protection(table)?;
        self.validate_protection(key)?;
        self.validate_value(value)?;
        self.collector.with_mut(table.reference, |table| {
            table.set(&Value::String(key.reference), value);
        })
    }

    /// Attach a validated key/value pair to a protected Table.
    ///
    /// This covers numeric argument-table keys and other VM-owned entries
    /// whose key is not a newly allocated string.
    pub fn set_table_entry(
        &mut self,
        table: &Rooted<'scope, Table>,
        key: &Value,
        value: &Value,
    ) -> Result<(), GcRefValidationError> {
        self.validate_protection(table)?;
        self.validate_value(key)?;
        self.validate_value(value)?;
        self.collector
            .with_mut(table.reference, |table| table.set(key, value))
    }

    /// Attach a protected string value under a validated arbitrary key.
    pub fn set_table_entry_string(
        &mut self,
        table: &Rooted<'scope, Table>,
        key: &Value,
        value: &Rooted<'scope, GcString>,
    ) -> Result<(), GcRefValidationError> {
        self.validate_protection(value)?;
        self.validate_protection(table)?;
        self.validate_value(key)?;
        self.collector.with_mut(table.reference, |table| {
            table.set(key, &Value::String(value.reference));
        })
    }

    /// Attach a protected string value to a protected Table.
    pub fn set_table_string(
        &mut self,
        table: &Rooted<'scope, Table>,
        key: &Rooted<'scope, GcString>,
        value: &Rooted<'scope, GcString>,
    ) -> Result<(), GcRefValidationError> {
        self.validate_protection(value)?;
        self.set_table_value(table, key, &Value::String(value.reference))
    }

    /// Attach a protected child Table to a protected parent Table.
    pub fn set_table_table(
        &mut self,
        table: &Rooted<'scope, Table>,
        key: &Rooted<'scope, GcString>,
        value: &Rooted<'scope, Table>,
    ) -> Result<(), GcRefValidationError> {
        self.validate_protection(value)?;
        self.set_table_value(table, key, &Value::Table(value.reference))
    }

    /// Attach a protected Function to a protected Table.
    pub fn set_table_function(
        &mut self,
        table: &Rooted<'scope, Table>,
        key: &Rooted<'scope, GcString>,
        value: &Rooted<'scope, Function>,
    ) -> Result<(), GcRefValidationError> {
        self.validate_protection(value)?;
        self.set_table_value(table, key, &Value::Function(value.reference))
    }

    /// Attach a protected Userdata to a protected Table.
    pub fn set_table_userdata(
        &mut self,
        table: &Rooted<'scope, Table>,
        key: &Rooted<'scope, GcString>,
        value: &Rooted<'scope, Userdata>,
    ) -> Result<(), GcRefValidationError> {
        self.validate_protection(value)?;
        self.set_table_value(table, key, &Value::Userdata(value.reference))
    }

    /// Set a protected Table's metatable to another protected Table.
    pub fn set_table_metatable(
        &mut self,
        table: &Rooted<'scope, Table>,
        metatable: Option<&Rooted<'scope, Table>>,
    ) -> Result<(), GcRefValidationError> {
        self.validate_protection(table)?;
        let metatable = match metatable {
            Some(metatable) => {
                self.validate_protection(metatable)?;
                Some(metatable.reference)
            }
            None => None,
        };
        self.collector
            .with_mut(table.reference, |table| table.set_metatable(metatable))
    }

    /// Set a protected Userdata's metatable to a protected Table.
    pub fn set_userdata_metatable(
        &mut self,
        userdata: &Rooted<'scope, Userdata>,
        metatable: Option<&Rooted<'scope, Table>>,
    ) -> Result<(), GcRefValidationError> {
        self.validate_protection(userdata)?;
        let metatable = match metatable {
            Some(metatable) => {
                self.validate_protection(metatable)?;
                Some(metatable.reference)
            }
            None => None,
        };
        self.collector.with_mut(userdata.reference, |userdata| {
            userdata.set_metatable(metatable)
        })
    }

    /// Set a protected Userdata's metatable to an already-live Table.
    pub fn set_userdata_metatable_reference(
        &mut self,
        userdata: &Rooted<'scope, Userdata>,
        metatable: Option<GcRef<Table>>,
    ) -> Result<(), GcRefValidationError> {
        self.validate_protection(userdata)?;
        if let Some(metatable) = metatable {
            self.collector.validate_ref(metatable)?;
        }
        self.collector.with_mut(userdata.reference, |userdata| {
            userdata.set_metatable(metatable)
        })
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

    /// Check that a protected Lua Function retains the protected Proto.
    pub fn function_reaches_proto(
        &self,
        function: &Rooted<'scope, Function>,
        proto: &Rooted<'scope, Proto>,
    ) -> Result<bool, GcRefValidationError> {
        self.validate_protection(function)?;
        self.validate_protection(proto)?;
        self.collector.with_ref(function.reference, |function| {
            function.proto() == Some(proto.reference)
        })
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

    /// Publish a protected string Value while its temporary root remains
    /// installed for the entire callback.
    ///
    /// # Safety
    ///
    /// On normal return, `publish` must install the supplied Value in an
    /// independently traced owner or consume it synchronously.
    pub unsafe fn publish_string_value<R>(
        &mut self,
        rooted: Rooted<'scope, GcString>,
        publish: impl FnOnce(Value) -> R,
    ) -> Result<R, GcRefValidationError> {
        self.validate_protection(&rooted)?;
        let result = publish(Value::String(rooted.reference));
        self.release_owned_root(rooted.temporary_root_id);
        Ok(result)
    }

    /// Publish a protected Table Value while its temporary root remains
    /// installed for the entire callback.
    ///
    /// # Safety
    ///
    /// On normal return, `publish` must install the supplied Value in an
    /// independently traced owner or consume it synchronously.
    pub unsafe fn publish_table_value<R>(
        &mut self,
        rooted: Rooted<'scope, Table>,
        publish: impl FnOnce(Value) -> R,
    ) -> Result<R, GcRefValidationError> {
        self.validate_protection(&rooted)?;
        let result = publish(Value::Table(rooted.reference));
        self.release_owned_root(rooted.temporary_root_id);
        Ok(result)
    }

    /// Publish a protected Userdata Value while its temporary root remains
    /// installed for the entire callback.
    ///
    /// # Safety
    ///
    /// On normal return, `publish` must install the supplied Value in an
    /// independently traced owner or consume it synchronously.
    pub unsafe fn publish_userdata_value<R>(
        &mut self,
        rooted: Rooted<'scope, Userdata>,
        publish: impl FnOnce(Value) -> R,
    ) -> Result<R, GcRefValidationError> {
        self.validate_protection(&rooted)?;
        let result = publish(Value::Userdata(rooted.reference));
        self.release_owned_root(rooted.temporary_root_id);
        Ok(result)
    }

    /// Publish a protected Upvalue reference into a traced non-`Value` owner.
    ///
    /// # Safety
    ///
    /// On normal return, `publish` must install the supplied reference in an
    /// independently traced owner, such as a LuaState open-Upvalue set or a
    /// protected Function. It must not escape only through an unrooted local.
    pub unsafe fn publish_upvalue_reference<R>(
        &mut self,
        rooted: Rooted<'scope, Upvalue>,
        publish: impl FnOnce(GcRef<Upvalue>) -> R,
    ) -> Result<R, GcRefValidationError> {
        self.validate_protection(&rooted)?;
        let result = publish(rooted.reference);
        self.release_owned_root(rooted.temporary_root_id);
        Ok(result)
    }

    /// Protect an existing result slice while a callback consumes or publishes
    /// every collectable value.
    ///
    /// Each collectable entry receives its own exact-id temporary root. This
    /// deliberately accepts duplicate references: exact identities make
    /// nested/result-buffer cleanup independent of explicit-root ownership.
    ///
    /// # Safety
    ///
    /// Before returning normally, `publish` must either consume every supplied
    /// value synchronously or install it in an independently traced owner. It
    /// must not let collectable values escape only through unrooted locals or
    /// return values.
    pub unsafe fn publish_value_slice<R>(
        &mut self,
        values: &[Value],
        publish: impl FnOnce(&mut GarbageCollector, &[Value]) -> R,
    ) -> Result<R, GcRefValidationError> {
        for value in values {
            match value {
                Value::String(reference) => {
                    let _ = self.protect(*reference)?;
                }
                Value::Table(reference) => {
                    let _ = self.protect(*reference)?;
                }
                Value::Function(reference) => {
                    let _ = self.protect(*reference)?;
                }
                Value::Userdata(reference) => {
                    let _ = self.protect(*reference)?;
                }
                Value::Thread(reference) => {
                    let _ = self.protect(*reference)?;
                }
                Value::Nil | Value::Boolean(_) | Value::Number(_) | Value::LightUserdata(_) => {}
            }
        }
        Ok(publish(self.collector, values))
    }

    /// Run a nested publication transaction while retaining all outer roots.
    pub fn with_nested<R>(
        &mut self,
        publish: impl for<'nested> FnOnce(&mut PublicationTxn<'nested>) -> R,
    ) -> R {
        let mut nested = PublicationTxn::new(&mut *self.collector);
        publish(&mut nested)
    }

    /// Run the collector's non-destructive mark seed while this transaction's
    /// lexical roots are active.
    pub fn trace_mark_only(&mut self) -> MarkRootSeedReport {
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

    fn validate_value(&self, value: &Value) -> Result<(), GcRefValidationError> {
        match value {
            Value::String(value) => self.collector.validate_ref(*value).map(|_| ()),
            Value::Table(value) => self.collector.validate_ref(*value).map(|_| ()),
            Value::Function(value) => self.collector.validate_ref(*value).map(|_| ()),
            Value::Userdata(value) => self.collector.validate_ref(*value).map(|_| ()),
            Value::Thread(value) => self.collector.validate_ref(*value).map(|_| ()),
            Value::Nil | Value::Boolean(_) | Value::Number(_) | Value::LightUserdata(_) => Ok(()),
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
    fn result_slice_is_rooted_for_callback_and_released_after_consumption() {
        let mut collector = GarbageCollector::new();
        let string = collector.create(GcString::from_bytes(b"result"));
        let table = collector.create(Table::new());
        let values = [
            Value::String(string),
            Value::Table(table),
            Value::Number(7.0),
        ];

        collector.with_publication(|transaction| {
            // SAFETY: the callback consumes the values synchronously and does
            // not let either collectable reference escape.
            let kinds = unsafe {
                transaction.publish_value_slice(&values, |collector, values| {
                    assert_eq!(collector.temporary_root_count(), 2);
                    let report = collector.begin_mark_only();
                    assert_eq!(report.temporary_seeded, 2);
                    values
                        .iter()
                        .map(|value| match value {
                            Value::String(_) => "string",
                            Value::Table(_) => "table",
                            Value::Number(_) => "number",
                            _ => "other",
                        })
                        .collect::<Vec<_>>()
                })
            }
            .expect("live result values are accepted");
            assert_eq!(kinds, ["string", "table", "number"]);
        });

        assert_eq!(collector.temporary_root_count(), 0);
        assert_eq!(collector.rejected_temporary_root_release_count(), 0);
    }

    #[test]
    fn result_slice_foreign_failure_and_panic_cleanup_exact_roots() {
        let mut owner = GarbageCollector::new();
        let local = owner.create(Table::new());
        let mut foreign = GarbageCollector::new();
        let foreign_value = foreign.create(Table::new());
        let mut callback_ran = false;

        owner.with_publication(|transaction| {
            // SAFETY: validation fails before the callback can run.
            let result = unsafe {
                transaction.publish_value_slice(
                    &[Value::Table(local), Value::Table(foreign_value)],
                    |_, _| callback_ran = true,
                )
            };
            assert!(result.is_err());
            assert!(!callback_ran);
            assert_eq!(transaction.active_temporary_root_count(), 1);
        });
        assert_eq!(owner.temporary_root_count(), 0);

        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            owner.with_publication(|transaction| {
                // SAFETY: the callback never returns normally.
                unsafe {
                    transaction.publish_value_slice(&[Value::Table(local)], |_, _| {
                        panic!("injected result publication failure");
                    })
                }
                .expect("panic occurs inside result publication");
            });
        }));
        assert!(unwind.is_err());
        assert_eq!(owner.temporary_root_count(), 0);
        assert_eq!(owner.rejected_temporary_root_release_count(), 0);
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
