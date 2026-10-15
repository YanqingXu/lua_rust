//! 只打包已得到的 body outcome，不执行 Lua，也不模拟临时 roots 或 yield depth。
use lua_core::{ByteString, Value};
use lua_vm::{VmError, VmErrorKind};

pub type Outcome = Result<Vec<Value>, VmError>;
pub type Packer = fn(Outcome) -> Vec<Value>;
pub type XPacker = fn(Outcome, &mut dyn FnMut(Value) -> Outcome) -> Vec<Value>;

pub fn text(bytes: &[u8]) -> Value {
    Value::String(ByteString::new(bytes))
}

pub fn error_text(error: VmError) -> Value {
    text(error.to_string().as_bytes())
}

pub fn body_error() -> VmError {
    VmError {
        kind: VmErrorKind::Runtime,
        pc: 7,
        line: 3,
        message: "body failed".to_owned(),
        traceback: vec!["body caller".to_owned()],
    }
}

pub fn handler_error() -> VmError {
    VmError {
        kind: VmErrorKind::Type,
        pc: 11,
        line: 5,
        message: "handler failed".to_owned(),
        traceback: Vec::new(),
    }
}
