use crate::{
    expected,
    model::{InstallError, Installed, Installer},
    real,
};
use lua_core::{ByteString, MathFunction, NativeFunction, Value};
use lua_stdlib::{FunctionSpec, LibrarySpec, BASE_FUNCTIONS, LIBRARIES};

pub fn complete_catalog(installer: Installer) {
    let installed = installer(BASE_FUNCTIONS, LIBRARIES).unwrap();
    let actual = installed.snapshot().unwrap();
    let expected = expected::bindings();
    for ((scope, name), function) in &expected {
        assert_eq!(
            actual.get(&(scope.clone(), name.clone())),
            Some(function),
            "missing/wrong binding: {}.{}",
            String::from_utf8_lossy(scope),
            String::from_utf8_lossy(name)
        );
    }
    assert_eq!(actual.len(), expected.len(), "unexpected public binding");
    assert_effects(&installed);
    assert_eq!(installed.lookup(b"", b"gc_collect").unwrap(), Value::Nil);
    namespace_contract(installer);
}

fn namespace_contract(installer: Installer) {
    const LEFT: FunctionSpec = FunctionSpec {
        name: b"same",
        function: NativeFunction::Math(MathFunction::Abs),
    };
    const RIGHT: FunctionSpec = FunctionSpec {
        name: b"same",
        function: NativeFunction::Math(MathFunction::Sqrt),
    };
    let duplicate = [LibrarySpec {
        name: b"left",
        functions: &[LEFT, RIGHT],
    }];
    assert!(
        matches!(installer(&[], &duplicate), Err(InstallError::Duplicate { scope, name }) if scope == b"left" && name == b"same")
    );
    assert!(
        matches!(installer(&[LEFT, RIGHT], &[]), Err(InstallError::Duplicate { scope, name }) if scope == b"_G" && name == b"same")
    );
    let valid = [
        LibrarySpec {
            name: b"left",
            functions: &[LEFT],
        },
        LibrarySpec {
            name: b"right",
            functions: &[RIGHT],
        },
    ];
    let installed = installer(&[], &valid).unwrap();
    assert_eq!(
        installed.snapshot().unwrap().len(),
        2,
        "installer must honor its input catalog"
    );
    assert_eq!(
        real::installed_call(&installed, b"left", b"same", b"local f=0; return f(81)"),
        [Value::Number(81.0)]
    );
    assert_eq!(
        real::installed_call(&installed, b"right", b"same", b"local f=0; return f(81)"),
        [Value::Number(9.0)]
    );
}

pub fn assert_effects(installed: &Installed) {
    assert_eq!(
        real::installed_effects(installed),
        vec![
            vec![Value::Number(42.0)],
            vec![Value::Number(9.0)],
            vec![Value::Number(3.0)],
            vec![Value::String(ByteString::new(b"3,1,2".as_slice()))],
            vec![Value::Boolean(true), Value::Number(42.0)],
        ]
    );
}
