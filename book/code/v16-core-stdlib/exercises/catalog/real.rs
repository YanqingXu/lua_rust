//! 安装实验得到的稳定 ID 交给真实 VM 分派；不复制任何 builtin 函数体。
use crate::model::Installed;
use lua_compiler::compile;
use lua_core::{NativeFunction, Value};
use lua_vm::{Vm, VmError};

pub fn invoke_id(function: NativeFunction, source: &[u8]) -> Result<Vec<Value>, VmError> {
    let mut proto = compile(source).expect("fixed probe must compile");
    let markers = proto
        .constants
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (*value == Value::Number(0.0)).then_some(index))
        .collect::<Vec<_>>();
    assert_eq!(
        markers.len(),
        1,
        "fixed probe has exactly one numeric marker"
    );
    proto.constants[markers[0]] = Value::NativeFunction(function);
    Vm::new().execute_multi(&proto)
}

pub fn installed_call(
    installed: &Installed,
    scope: &[u8],
    name: &[u8],
    source: &[u8],
) -> Vec<Value> {
    let Value::NativeFunction(function) = installed.lookup(scope, name).unwrap() else {
        panic!("fixed probe requires an installed builtin");
    };
    invoke_id(function, source).unwrap()
}

// ANCHOR: catalog_real_calls
pub fn installed_effects(installed: &Installed) -> Vec<Vec<Value>> {
    vec![
        installed_call(installed, b"", b"tonumber", br#"local f=0; return f('42')"#),
        installed_call(installed, b"math", b"sqrt", b"local f=0; return f(81)"),
        installed_call(
            installed,
            b"string",
            b"len",
            br#"local f=0; return f('A\000\255')"#,
        ),
        installed_call(
            installed,
            b"table",
            b"concat",
            br#"local f=0; return f({3,1,2}, ',')"#,
        ),
        installed_call(
            installed,
            b"coroutine",
            b"create",
            br#"
            local f=0
            local co=f(function() return 42 end)
            return coroutine.resume(co)
        "#,
        ),
    ]
}
// ANCHOR_END: catalog_real_calls

pub fn public_vm_effects() -> Vec<Value> {
    let proto = compile(
        br#"
        local binary=string.char(65,0,255)
        local co=coroutine.create(function() return 42 end)
        local ok, answer=coroutine.resume(co)
        return tonumber('42'), math.sqrt(81), string.len(binary),
          table.concat({3,1,2}, ','), ok, answer
    "#,
    )
    .unwrap();
    Vm::new().execute_multi(&proto).unwrap()
}

pub fn diagnostic_is_separately_installed() -> Value {
    let proto =
        compile(b"return type(gc_collect()) == 'number' and type(gc_step) == 'function'").unwrap();
    Vm::new().execute(&proto).unwrap()
}
