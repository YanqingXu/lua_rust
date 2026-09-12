#![forbid(unsafe_code)]

use lua_core::gc::collector::GcRefValidationError;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::Runtime;

#[test]
fn source_execution_and_result_callbacks_preserve_state_borrow_lifetimes() {
    let mut runtime = Runtime::new();
    let chunk = lua_compiler::parser::Parser::new("return { 7 }")
        .parse()
        .unwrap();
    let proto = {
        let mut parts = runtime.parts_mut().unwrap();
        let (_, gc, strings) = parts.split_mut();
        let proto = lua_compiler::codegen::CodeGenerator::new_with_pool(gc, strings)
            .generate(&chunk, "<state-lifetime>")
            .unwrap();
        gc.create(proto)
    };
    let mut observed = false;
    runtime
        .execute_proto_with_args(proto, Vec::new(), |state, values| {
            let [Value::Table(table)] = values else {
                panic!("expected one table")
            };
            assert_eq!(
                state.with_gc_ref(*table, |table| table.get_array(1)),
                Ok(Value::Number(7.0))
            );
            observed = true;
        })
        .unwrap();
    assert!(observed);
    assert_eq!(runtime.close().unwrap().remaining_objects, 0);
}

#[test]
fn explicit_parts_reject_dynamic_access_before_a_callback_can_collect() {
    let mut runtime = Runtime::new();
    {
        let mut parts = runtime.parts_mut().unwrap();
        let (state, gc, strings) = parts.split_mut();
        let table = gc.create(Table::new());
        let mut callback_ran = false;
        let result = state.with_gc_ref(table, |borrowed| {
            callback_ran = true;
            gc.collect(strings);
            borrowed.length()
        });
        assert_eq!(result, Err(GcRefValidationError::CollectorUnavailable));
        assert!(!callback_ran);
        assert_eq!(gc.with_ref(table, Table::length), Ok(0));
        assert!(state.active_gc_ptr().is_none());
        assert!(state.active_string_pool_ptr().is_none());
    }
    assert_eq!(runtime.close().unwrap().remaining_objects, 0);
}

#[test]
fn explicit_and_dynamic_contexts_can_alternate_without_aliasing_services() {
    let mut runtime = Runtime::new();
    {
        let mut parts = runtime.parts_mut().unwrap();
        let table = {
            let (_, gc, _) = parts.split_mut();
            gc.create_root(Table::new())
        };
        parts.with_context_gc(|state, gc| {
            assert!(state.active_gc_ptr().is_none());
            assert!(state.active_string_pool_ptr().is_some());
            assert_eq!(
                state.with_gc_ref(table, Table::length),
                Err(GcRefValidationError::CollectorUnavailable)
            );
            gc.with_mut(table, |table| {
                table.set(&Value::Number(1.0), &Value::Number(42.0));
            })
            .unwrap();
        });
        assert!(parts.state().active_gc_ptr().is_none());
        parts.with_context(|state| {
            assert_eq!(state.with_gc_ref(table, Table::length), Ok(1));
            assert!(state.active_string_pool_ptr().is_some());
        });
        assert!(parts.state().active_gc_ptr().is_none());
        let (_, gc, strings) = parts.split_mut();
        gc.remove_root(table);
        gc.collect(strings);
        assert!(!gc.contains_registered(table));
    }
    assert_eq!(runtime.close().unwrap().remaining_objects, 0);
}

#[test]
fn callback_unwind_releases_context_and_keeps_the_runtime_usable() {
    let mut runtime = Runtime::new();
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut parts = runtime.parts_mut().unwrap();
        parts.with_context(|state| {
            assert!(state.active_gc_ptr().is_some());
            panic!("injected host callback error");
        });
    }));
    assert!(unwind.is_err());
    assert_eq!(runtime.active_execution_count(), 0);
    assert!(runtime.main_state().unwrap().active_gc_ptr().is_none());
    {
        let mut parts = runtime.parts_mut().unwrap();
        parts.with_context(|state| state.push_number(7.0));
        assert_eq!(parts.state().stack.at(0), Some(&Value::Number(7.0)));
    }
    assert_eq!(runtime.close().unwrap().remaining_objects, 0);
}
