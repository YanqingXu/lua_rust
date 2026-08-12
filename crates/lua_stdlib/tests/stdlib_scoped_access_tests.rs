const BASE_SOURCE: &str = include_str!("../src/base.rs");
const CATALOG_SOURCE: &str = include_str!("../src/catalog.rs");
const COROUTINE_SOURCE: &str = include_str!("../src/coroutine.rs");
const PACKAGE_SOURCE: &str = include_str!("../src/package.rs");
const DEBUG_SOURCE: &str = include_str!("../src/debug.rs");
const IO_SOURCE: &str = include_str!("../src/io.rs");
const LIB_SOURCE: &str = include_str!("../src/lib.rs");
const MATH_SOURCE: &str = include_str!("../src/math.rs");
const OS_SOURCE: &str = include_str!("../src/os.rs");
const REGISTRATION_SOURCE: &str = include_str!("../src/registration.rs");
const STRING_SOURCE: &str = include_str!("../src/string.rs");
const TABLE_SOURCE: &str = include_str!("../src/table.rs");

fn production_prefix(source: &str) -> &str {
    source
        .split("#[cfg(test)]")
        .next()
        .expect("standard-library source has a production prefix")
}

#[test]
fn base_package_and_debug_function_table_reads_are_scoped() {
    let base = production_prefix(BASE_SOURCE);
    for forbidden in ["table_ref.as_ref()", "func_ref.as_ref()"] {
        assert!(
            !base.contains(forbidden),
            "base library regressed to direct Function/Table access: {forbidden}"
        );
    }
    assert!(base.matches("with_gc_ref(").count() >= 7);

    let package = production_prefix(PACKAGE_SOURCE);
    for forbidden in ["module_ref.as_ref()", "func_ref.as_ref()", "table.as_ref()"] {
        assert!(
            !package.contains(forbidden),
            "package library regressed to direct Function/Table access: {forbidden}"
        );
    }
    assert!(package.contains("with_gc_ref(*func_ref, Function::env)"));
    assert!(package.matches(".with_ref(").count() >= 2);

    let debug = production_prefix(DEBUG_SOURCE);
    for forbidden in ["func_ref.as_ref()", "env.as_ref()"] {
        assert!(
            !debug.contains(forbidden),
            "debug library regressed to direct Function/Table access: {forbidden}"
        );
    }
    assert!(debug.matches("with_gc_ref(").count() >= 5);
    assert!(debug.contains("fn function_key_in_table("));
}

#[test]
fn all_stdlib_managed_object_reads_require_a_scoped_accessor() {
    for (name, source) in [
        ("base", BASE_SOURCE),
        ("catalog", CATALOG_SOURCE),
        ("coroutine", COROUTINE_SOURCE),
        ("debug", DEBUG_SOURCE),
        ("io", IO_SOURCE),
        ("lib", LIB_SOURCE),
        ("math", MATH_SOURCE),
        ("os", OS_SOURCE),
        ("package", PACKAGE_SOURCE),
        ("registration", REGISTRATION_SOURCE),
        ("string", STRING_SOURCE),
        ("table", TABLE_SOURCE),
    ] {
        for line in production_prefix(source).lines() {
            assert!(
                !(line.contains("unsafe")
                    && (line.contains(".as_ref()") || line.contains(".as_mut()"))),
                "{name} contains an unaudited unsafe object dereference: {line}"
            );
        }
    }
}
