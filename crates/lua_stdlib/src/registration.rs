//! Transactional publication helpers shared by standard-library builders.

use lua_core::function::{CFunction, RuntimeNativeFunction};
use lua_core::gc::collector::{GarbageCollector, GcRefValidationError};
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc::publication::{PublicationTxn, Rooted};
use lua_core::gc_string::GcString;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::state::LuaState;

pub(crate) fn rooted_bytes<'scope>(
    state: &LuaState,
    transaction: &mut PublicationTxn<'scope>,
    bytes: &[u8],
) -> Result<Rooted<'scope, GcString>, GcRefValidationError> {
    if let Some(pool) = state.string_pool {
        // SAFETY: LuaState::string_pool is installed from the live host-owned
        // StringPool for the duration of standard-library execution.
        transaction.intern_bytes(unsafe { &mut *pool }, bytes)
    } else {
        Ok(transaction.alloc(GcString::from_bytes(bytes)))
    }
}

pub(crate) fn register_c_function(
    state: &LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    name: &[u8],
    function: CFunction,
    environment: Option<GcRef<Table>>,
) -> Result<(), GcRefValidationError> {
    gc.with_publication(|transaction| {
        let table = transaction.protect(table)?;
        let name = rooted_bytes(state, transaction, name)?;
        let function = transaction.alloc_c_function(function);
        transaction.set_function_environment(&function, environment)?;
        transaction.set_table_function(&table, &name, &function)
    })
}

pub(crate) fn register_c_function_aliases(
    state: &LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    names: &[&[u8]],
    function: CFunction,
    environment: Option<GcRef<Table>>,
) -> Result<(), GcRefValidationError> {
    gc.with_publication(|transaction| {
        let table = transaction.protect(table)?;
        let function = transaction.alloc_c_function(function);
        transaction.set_function_environment(&function, environment)?;
        for name in names {
            let name = rooted_bytes(state, transaction, name)?;
            transaction.set_table_function(&table, &name, &function)?;
        }
        Ok(())
    })
}

pub(crate) fn register_runtime_native(
    state: &LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    name: &[u8],
    operation: RuntimeNativeFunction,
) -> Result<(), GcRefValidationError> {
    gc.with_publication(|transaction| {
        let table = transaction.protect(table)?;
        let name = rooted_bytes(state, transaction, name)?;
        let function = transaction.alloc_runtime_native_function(operation);
        transaction.set_table_function(&table, &name, &function)
    })
}

pub(crate) fn set_value(
    state: &LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    key: &[u8],
    value: &Value,
) -> Result<(), GcRefValidationError> {
    gc.with_publication(|transaction| {
        let table = transaction.protect(table)?;
        let key = rooted_bytes(state, transaction, key)?;
        transaction.set_table_value(&table, &key, value)
    })
}

pub(crate) fn set_string(
    state: &LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    key: &[u8],
    value: &[u8],
) -> Result<(), GcRefValidationError> {
    gc.with_publication(|transaction| {
        let table = transaction.protect(table)?;
        let key = rooted_bytes(state, transaction, key)?;
        let value = rooted_bytes(state, transaction, value)?;
        transaction.set_table_string(&table, &key, &value)
    })
}

pub(crate) fn set_metatable(
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    metatable: GcRef<Table>,
) -> Result<(), GcRefValidationError> {
    gc.with_publication(|transaction| {
        let table = transaction.protect(table)?;
        let metatable = transaction.protect(metatable)?;
        transaction.set_table_metatable(&table, Some(&metatable))
    })
}

pub(crate) fn publish_new_table(
    state: &LuaState,
    gc: &mut GarbageCollector,
    parent: GcRef<Table>,
    key: &[u8],
) -> Result<GcRef<Table>, String> {
    gc.with_publication(|transaction| {
        let parent = transaction
            .protect(parent)
            .map_err(|error| format!("invalid library parent Table: {error}"))?;
        let key = rooted_bytes(state, transaction, key)
            .map_err(|error| format!("invalid library key: {error}"))?;
        let child = transaction.alloc(Table::new());
        transaction
            .set_table_table(&parent, &key, &child)
            .map_err(|error| format!("invalid library Table publication: {error}"))
    })?;

    match table_value_by_bytes(gc, parent, key)? {
        Value::Table(table) => Ok(table),
        _ => Err("published library Table is not reachable from its parent".to_string()),
    }
}

pub(crate) fn ensure_table(
    state: &LuaState,
    gc: &mut GarbageCollector,
    parent: GcRef<Table>,
    key: &[u8],
) -> Result<GcRef<Table>, String> {
    if let Value::Table(table) = table_value_by_bytes(gc, parent, key)? {
        return Ok(table);
    }
    publish_new_table(state, gc, parent, key)
}

pub(crate) fn table_value_by_bytes(
    gc: &GarbageCollector,
    table: GcRef<Table>,
    key: &[u8],
) -> Result<Value, String> {
    let entries = gc
        .with_ref(table, |table| {
            table
                .hash_entries()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Vec<_>>()
        })
        .map_err(|error| format!("invalid library Table: {error}"))?;

    for (candidate, value) in entries {
        let Value::String(candidate) = candidate else {
            continue;
        };
        let matches = gc
            .with_ref(candidate, |candidate| candidate.as_bytes() == key)
            .map_err(|error| format!("invalid library key: {error}"))?;
        if matches {
            return Ok(value);
        }
    }
    Ok(Value::Nil)
}

pub(crate) fn push_string(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    bytes: &[u8],
) -> Result<(), GcRefValidationError> {
    gc.with_publication(|transaction| {
        let string = rooted_bytes(state, transaction, bytes)?;
        // SAFETY: the callback installs the string on the active Lua stack
        // before its temporary root is released.
        unsafe {
            transaction.publish_string_value(string, |value| {
                state.push_value(value);
            })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lua_core::string_pool::StringPool;

    unsafe extern "C" fn fixture(_state: *mut std::ffi::c_void) -> i32 {
        0
    }

    fn state_with_services(
        gc: &mut GarbageCollector,
        pool: &mut StringPool,
    ) -> (LuaState, GcRef<Table>) {
        let global = gc.create_root(Table::new());
        let mut state = LuaState::new();
        state.global_table = Some(global);
        state.gc = Some(gc);
        state.string_pool = Some(pool);
        (state, global)
    }

    #[test]
    fn aliases_share_one_function_and_canonical_keys() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let (state, global) = state_with_services(&mut gc, &mut pool);
        let library = publish_new_table(&state, &mut gc, global, b"fixture")
            .expect("library table should publish");

        register_c_function_aliases(
            &state,
            &mut gc,
            library,
            &[&b"primary"[..], &b"alias"[..]],
            fixture,
            None,
        )
        .expect("aliases should publish");

        let primary =
            table_value_by_bytes(&gc, library, b"primary").expect("primary lookup should validate");
        let alias =
            table_value_by_bytes(&gc, library, b"alias").expect("alias lookup should validate");
        assert_eq!(primary, alias);
        assert!(matches!(primary, Value::Function(_)));
        assert!(pool.find_bytes(b"primary").is_some());
        assert!(pool.find_bytes(b"alias").is_some());
        assert!(pool.find_bytes(b"fixture").is_some());
        assert_eq!(gc.temporary_root_count(), 0);

        gc.mark();
        assert_eq!(gc.marked_object_count(), gc.object_count());
        let object_count = gc.object_count();
        gc.remove_root(global);
        gc.mark();
        assert_eq!(gc.sweep(&mut pool), object_count);
        assert_eq!(gc.object_count(), 0);
        assert!(pool.is_empty());
    }

    #[test]
    fn open_all_publishes_every_library_graph_from_the_global_root() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let (mut state, global) = state_with_services(&mut gc, &mut pool);

        crate::catalog::open_all(&mut state, &mut gc);

        for name in [
            b"math".as_slice(),
            b"io",
            b"os",
            b"string",
            b"table",
            b"coroutine",
            b"debug",
            b"package",
        ] {
            assert!(
                matches!(
                    table_value_by_bytes(&gc, global, name)
                        .expect("global library lookup should validate"),
                    Value::Table(_)
                ),
                "library {} should be globally published",
                String::from_utf8_lossy(name)
            );
        }
        let package = table_value_by_bytes(&gc, global, b"package")
            .expect("package lookup should validate")
            .as_table();
        assert!(matches!(
            table_value_by_bytes(&gc, package, b"loaded")
                .expect("package.loaded lookup should validate"),
            Value::Table(_)
        ));
        assert!(matches!(
            table_value_by_bytes(&gc, package, b"preload")
                .expect("package.preload lookup should validate"),
            Value::Table(_)
        ));
        assert_eq!(gc.temporary_root_count(), 0);
        gc.mark();
        assert_eq!(gc.rejected_mark_edge_count(), 0);
        assert_eq!(gc.marked_object_count(), gc.object_count());
    }

    #[test]
    fn foreign_library_edge_is_rejected_before_table_mutation() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let (state, global) = state_with_services(&mut gc, &mut pool);
        let mut foreign_gc = GarbageCollector::new();
        let foreign = foreign_gc.create(Table::new());

        let result = set_value(&state, &mut gc, global, b"foreign", &Value::Table(foreign));

        assert!(result.is_err());
        assert_eq!(gc.temporary_root_count(), 0);
        assert!(matches!(
            table_value_by_bytes(&gc, global, b"foreign")
                .expect("global lookup should remain valid"),
            Value::Nil
        ));
        gc.mark();
        assert_eq!(gc.marked_object_count(), 1);
        assert_eq!(gc.sweep(&mut pool), 1);
        assert!(pool.is_empty());
    }

    #[test]
    fn panic_after_partial_library_edges_releases_only_temporary_roots() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let (state, global) = state_with_services(&mut gc, &mut pool);

        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            gc.with_publication(|transaction| {
                let global = transaction.protect(global).expect("global should protect");
                let library = transaction.alloc(Table::new());
                let library_name = rooted_bytes(&state, transaction, b"fixture")
                    .expect("library name should root");
                transaction
                    .set_table_table(&global, &library_name, &library)
                    .expect("library edge should publish");

                let function_name = rooted_bytes(&state, transaction, b"partial")
                    .expect("function name should root");
                let function = transaction.alloc_c_function(fixture);
                transaction
                    .set_table_function(&library, &function_name, &function)
                    .expect("function edge should publish");
                assert!(transaction.active_temporary_root_count() >= 5);
                panic!("injected library registration failure");
            });
        }));

        assert!(unwind.is_err());
        assert_eq!(gc.temporary_root_count(), 0);
        assert_eq!(gc.rejected_temporary_root_release_count(), 0);
        gc.mark();
        assert_eq!(gc.marked_object_count(), gc.object_count());
        assert!(matches!(
            table_value_by_bytes(&gc, global, b"fixture")
                .expect("published prefix should remain valid"),
            Value::Table(_)
        ));

        let object_count = gc.object_count();
        gc.remove_root(global);
        gc.mark();
        assert_eq!(gc.sweep(&mut pool), object_count);
        assert_eq!(gc.object_count(), 0);
        assert!(pool.is_empty());
    }
}
