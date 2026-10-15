mod contracts;
mod expected;
mod faults;
mod model;
mod real;
mod solution;

use lua_core::{BaseFunction, MathFunction, NativeFunction, Value};
use lua_stdlib::{FunctionSpec, LibrarySpec, BASE_FUNCTIONS, LIBRARIES};
use model::{InstallError, Installed};

#[test]
fn catalog_installs_the_independent_public_contract() {
    contracts::complete_catalog(solution::install);
}

#[test]
fn faulty_installer_leaves_a_declared_function_missing() {
    let before = faults::omit_sqrt(BASE_FUNCTIONS, LIBRARIES).unwrap();
    assert_eq!(before.lookup(b"math", b"sqrt").unwrap(), Value::Nil);
    assert_ne!(before.snapshot().unwrap(), expected::bindings());
    assert!(LIBRARIES
        .iter()
        .find(|library| library.name == b"math")
        .unwrap()
        .functions
        .iter()
        .any(|spec| spec.name == b"sqrt"));
}

#[test]
fn faulty_id_is_callable_but_produces_the_wrong_result() {
    let before = faults::wrong_sqrt_id(BASE_FUNCTIONS, LIBRARIES).unwrap();
    assert_eq!(
        before.lookup(b"math", b"sqrt").unwrap(),
        Value::NativeFunction(NativeFunction::Math(MathFunction::Abs))
    );
    assert_eq!(
        real::installed_call(&before, b"math", b"sqrt", b"local f=0; return f(81)"),
        [Value::Number(81.0)]
    );
    assert_ne!(before.snapshot().unwrap(), expected::bindings());
}

#[test]
fn teaching_preflight_rejects_duplicates_within_one_namespace() {
    const DUPLICATE: &[FunctionSpec] = &[
        FunctionSpec {
            name: b"same",
            function: NativeFunction::Math(MathFunction::Abs),
        },
        FunctionSpec {
            name: b"same",
            function: NativeFunction::Math(MathFunction::Sqrt),
        },
    ];
    let libraries = [LibrarySpec {
        name: b"math",
        functions: DUPLICATE,
    }];
    assert_duplicate(solution::install(&[], &libraries), b"math", b"same");
    assert_duplicate(solution::install(DUPLICATE, &[]), b"_G", b"same");
    let one = LibrarySpec {
        name: b"math",
        functions: &DUPLICATE[..1],
    };
    let repeated_library = [one, one];
    assert_duplicate(solution::install(&[], &repeated_library), b"_G", b"math");
    const GLOBAL: &[FunctionSpec] = &[FunctionSpec {
        name: b"math",
        function: NativeFunction::Base(BaseFunction::Type),
    }];
    assert_duplicate(solution::install(GLOBAL, &libraries), b"_G", b"math");
}

fn assert_duplicate(result: Result<Installed, InstallError>, scope: &[u8], name: &[u8]) {
    assert!(
        matches!(result, Err(InstallError::Duplicate { scope: actual_scope, name: actual_name }) if actual_scope == scope && actual_name == name)
    );
}

#[test]
fn different_libraries_can_publish_the_same_member_name() {
    const LEFT: &[FunctionSpec] = &[FunctionSpec {
        name: b"same",
        function: NativeFunction::Math(MathFunction::Abs),
    }];
    const RIGHT: &[FunctionSpec] = &[FunctionSpec {
        name: b"same",
        function: NativeFunction::Math(MathFunction::Sqrt),
    }];
    let libraries = [
        LibrarySpec {
            name: b"left",
            functions: LEFT,
        },
        LibrarySpec {
            name: b"right",
            functions: RIGHT,
        },
    ];
    let installed = solution::install(&[], &libraries).unwrap();
    assert_eq!(
        real::installed_call(&installed, b"left", b"same", b"local f=0; return f(81)"),
        [Value::Number(81.0)]
    );
    assert_eq!(
        real::installed_call(&installed, b"right", b"same", b"local f=0; return f(81)"),
        [Value::Number(9.0)]
    );
}

#[test]
fn independent_contract_detects_catalog_omission_and_wrong_id_mutations() {
    let expected = expected::bindings();
    assert_eq!(
        model::declared_bindings(BASE_FUNCTIONS, LIBRARIES).unwrap(),
        expected
    );
    let missing_math = LIBRARIES
        .iter()
        .copied()
        .filter(|library| library.name != b"math")
        .collect::<Vec<_>>();
    let missing = model::declared_bindings(BASE_FUNCTIONS, &missing_math).unwrap();
    let key = (b"math".to_vec(), b"sqrt".to_vec());
    assert!(!missing.contains_key(&key) && expected.contains_key(&key));
    const WRONG: &[FunctionSpec] = &[FunctionSpec {
        name: b"sqrt",
        function: NativeFunction::Math(MathFunction::Abs),
    }];
    let mut wrong_math = LIBRARIES.to_vec();
    wrong_math
        .iter_mut()
        .find(|library| library.name == b"math")
        .unwrap()
        .functions = WRONG;
    let wrong = model::declared_bindings(BASE_FUNCTIONS, &wrong_math).unwrap();
    assert_eq!(wrong[&key], NativeFunction::Math(MathFunction::Abs));
    assert_ne!(wrong[&key], expected[&key]);
    assert_ne!(wrong, expected);
}

#[test]
fn installed_ids_execute_real_vm_bodies_with_expected_effects() {
    contracts::assert_effects(&solution::install(BASE_FUNCTIONS, LIBRARIES).unwrap());
}

#[test]
fn real_vm_catalog_effects_and_separate_diagnostics_remain_usable() {
    assert_eq!(
        real::public_vm_effects(),
        vec![
            Value::Number(42.0),
            Value::Number(9.0),
            Value::Number(3.0),
            Value::String(lua_core::ByteString::new(b"3,1,2".as_slice())),
            Value::Boolean(true),
            Value::Number(42.0)
        ]
    );
    assert_eq!(
        real::diagnostic_is_separately_installed(),
        Value::Boolean(true)
    );
}
