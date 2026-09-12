//! Scoped mutation of managed payloads without exposing their GC headers.

use std::ops::Deref;

use crate::function::Function;
use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc_string::GcString;
use crate::proto::{Instruction, Proto};
use crate::state_handle::StateHandle;
use crate::table::Table;
use crate::thread::{CoroutineStatus, Thread};
use crate::upvalue::Upvalue;
use crate::userdata::Userdata;
use crate::value::Value;

/// A collector-owned object's editable payload.
///
/// Read methods are available through `Deref`. Only the payload mutators below
/// are exposed: there is deliberately no `DerefMut`, constructor, or accessor
/// yielding `&mut T`. Replacing a managed allocation would overwrite its
/// intrusive GC header and invalidate the collector's ownership bookkeeping.
///
/// ```compile_fail
/// use lua_core::{gc::collector::GarbageCollector, table::Table};
/// let mut gc = GarbageCollector::new();
/// let table = gc.create(Table::new());
/// gc.with_mut(table, |table| **table = Table::new()).unwrap();
/// ```
///
/// The protected publication API has the same restriction.
///
/// ```compile_fail
/// use lua_core::{gc::collector::GarbageCollector, table::Table};
/// let mut gc = GarbageCollector::new();
/// gc.with_publication(|transaction| {
///     let table = transaction.alloc(Table::new());
///     transaction.with_mut(&table, |table| {
///         let _ = std::mem::replace(&mut **table, Table::new());
///     }).unwrap();
/// });
/// ```
///
/// Managed payload destruction belongs to the collector, not host borrows.
///
/// ```compile_fail
/// use lua_core::{gc::collector::GarbageCollector, userdata::Userdata};
/// let mut gc = GarbageCollector::new();
/// let userdata = gc.create(Userdata::new(0));
/// gc.with_mut(userdata, |userdata| userdata.run_destructor()).unwrap();
/// ```
pub struct GcWrite<'object, T: GcObject> {
    object: &'object mut T,
}

impl<'object, T: GcObject> GcWrite<'object, T> {
    pub(crate) fn new(object: &'object mut T) -> Self {
        Self { object }
    }
}

impl<T: GcObject> Deref for GcWrite<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.object
    }
}

// Keep the editable surface explicit. Forwarding only these methods prevents
// new whole-object or header accessors from silently becoming public write APIs.
macro_rules! payload_mutators {
    ($object:ty; $(fn $method:ident($($argument:ident: $kind:ty),* $(,)?) $(-> $result:ty)?;)*) => {
        impl GcWrite<'_, $object> {
            $(
                #[doc = concat!("See [`", stringify!($object), "::", stringify!($method), "`].")]
                pub fn $method(&mut self, $($argument: $kind),*) $(-> $result)? {
                    self.object.$method($($argument),*)
                }
            )*
        }
    };
}

payload_mutators! { Table;
    fn set(key: &Value, value: &Value);
    fn remove(key: &Value);
    fn clear();
    fn set_array(index: i32, value: &Value);
    fn set_metatable(metatable: Option<GcRef<Table>>);
    fn set_flags(flags: u8);
}

payload_mutators! { Function;
    fn set_upvalue(index: usize, upvalue: GcRef<Upvalue>);
    fn add_upvalue(upvalue: GcRef<Upvalue>);
    fn set_env(environment: Option<GcRef<Table>>);
    fn set_gc_list(list: Option<GcRef<Function>>);
}

payload_mutators! { Proto;
    fn set_num_params(count: u8);
    fn set_vararg(vararg: bool);
    fn set_vararg_flags(flags: u8);
    fn set_max_stack_size(size: u8);
    fn set_source(source: Option<GcRef<GcString>>);
    fn add_constant(value: Value) -> usize;
    fn append_constant_slot(value: Value) -> usize;
    fn add_instruction(instruction: Instruction) -> usize;
    fn set_instruction(index: usize, instruction: Instruction);
    fn code_mut() -> &mut Vec<Instruction>;
    fn add_line_info(line: i32);
    fn line_info_mut() -> &mut Vec<i32>;
    fn add_proto(proto: GcRef<Proto>) -> usize;
    fn add_loc_var(name: Option<GcRef<GcString>>, start: i32, end: i32, register: i32) -> usize;
    fn add_upvalue_name(name: GcRef<GcString>) -> usize;
    fn set_line_defined(line: i32);
    fn set_last_line_defined(line: i32);
    fn set_num_upvalues(count: u8);
    fn set_gc_list(list: Option<GcRef<Proto>>);
}

payload_mutators! { Thread;
    fn set_status(status: CoroutineStatus);
    fn set_state_handle(handle: StateHandle);
    fn set_caller(caller: Option<GcRef<Thread>>);
    fn mark_resumed();
    fn set_saved_nexeccalls(count: i32);
}

payload_mutators! { Upvalue;
    fn set_closed_value(value: Value);
    fn close(stack_value: Value);
}

payload_mutators! { Userdata;
    fn data_mut() -> &mut [u8];
    fn as_mut_ptr() -> *mut u8;
    fn set_metatable(metatable: Option<GcRef<Table>>);
}

impl GcWrite<'_, Userdata> {
    /// See [`Userdata::typed_mut`].
    pub fn typed_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.object.typed_mut()
    }

    /// See [`Userdata::data_as_mut`].
    ///
    /// # Safety
    /// The payload must contain a valid value of `T`, as required by
    /// [`Userdata::data_as_mut`].
    pub unsafe fn data_as_mut<T>(&mut self) -> Option<&mut T> {
        // SAFETY: the caller supplies the underlying typed-payload contract.
        unsafe { self.object.data_as_mut() }
    }

    /// See [`Userdata::write_typed`].
    ///
    /// # Safety
    /// The value must satisfy [`Userdata::write_typed`]'s construction and
    /// destruction contract.
    pub unsafe fn write_typed<T: 'static>(&mut self, value: T) {
        // SAFETY: the caller supplies the underlying construction contract.
        unsafe { self.object.write_typed(value) }
    }

    /// See [`Userdata::set_destructor`].
    ///
    /// # Safety
    /// The callback must satisfy [`Userdata::set_destructor`]'s payload and
    /// lifetime requirements.
    pub unsafe fn set_destructor(&mut self, destructor: unsafe fn(*mut u8)) {
        // SAFETY: the caller supplies the underlying destructor contract.
        unsafe { self.object.set_destructor(destructor) }
    }
}
