//! Host-facing contracts for the v0.16 core standard libraries.
//!
//! The VM still owns heap mutation and invocation. This crate owns the public
//! catalog and the coercion/error rules shared by every Rust builtin.

use lua_core::{
    BaseFunction, ByteString, DebugFunction, FileFunction, IoFunction, MathFunction,
    NativeFunction, OsFunction, PackageFunction, StringFunction, TableFunction, TableRef, Value,
};
use std::fmt;

#[derive(Clone, Copy, Debug)]
pub struct FunctionSpec {
    pub name: &'static [u8],
    pub function: NativeFunction,
}

#[derive(Clone, Copy, Debug)]
pub struct LibrarySpec {
    pub name: &'static [u8],
    pub functions: &'static [FunctionSpec],
}

pub const BASE_FUNCTIONS: &[FunctionSpec] = &[
    spec(b"assert", NativeFunction::Base(BaseFunction::Assert)),
    spec(
        b"collectgarbage",
        NativeFunction::Base(BaseFunction::CollectGarbage),
    ),
    spec(b"dofile", NativeFunction::Base(BaseFunction::DoFile)),
    spec(b"error", NativeFunction::Base(BaseFunction::Error)),
    spec(b"getmetatable", NativeFunction::GetMetatable),
    spec(b"ipairs", NativeFunction::IPairs),
    spec(b"loadfile", NativeFunction::Base(BaseFunction::LoadFile)),
    spec(
        b"loadstring",
        NativeFunction::Base(BaseFunction::LoadString),
    ),
    spec(b"next", NativeFunction::Next),
    spec(b"pairs", NativeFunction::Pairs),
    spec(b"pcall", NativeFunction::Base(BaseFunction::PCall)),
    spec(b"print", NativeFunction::Base(BaseFunction::Print)),
    spec(b"rawequal", NativeFunction::Base(BaseFunction::RawEqual)),
    spec(b"rawget", NativeFunction::RawGet),
    spec(b"rawset", NativeFunction::RawSet),
    spec(
        b"require",
        NativeFunction::Package(PackageFunction::Require),
    ),
    spec(b"select", NativeFunction::Base(BaseFunction::Select)),
    spec(b"setmetatable", NativeFunction::SetMetatable),
    spec(b"tonumber", NativeFunction::Base(BaseFunction::ToNumber)),
    spec(b"tostring", NativeFunction::ToString),
    spec(b"type", NativeFunction::Base(BaseFunction::Type)),
    spec(b"unpack", NativeFunction::Base(BaseFunction::Unpack)),
    spec(b"xpcall", NativeFunction::Base(BaseFunction::XPCall)),
];

pub const MATH_FUNCTIONS: &[FunctionSpec] = &[
    spec(b"abs", NativeFunction::Math(MathFunction::Abs)),
    spec(b"acos", NativeFunction::Math(MathFunction::Acos)),
    spec(b"asin", NativeFunction::Math(MathFunction::Asin)),
    spec(b"atan", NativeFunction::Math(MathFunction::Atan)),
    spec(b"atan2", NativeFunction::Math(MathFunction::Atan2)),
    spec(b"ceil", NativeFunction::Math(MathFunction::Ceil)),
    spec(b"cos", NativeFunction::Math(MathFunction::Cos)),
    spec(b"cosh", NativeFunction::Math(MathFunction::Cosh)),
    spec(b"deg", NativeFunction::Math(MathFunction::Deg)),
    spec(b"exp", NativeFunction::Math(MathFunction::Exp)),
    spec(b"floor", NativeFunction::Math(MathFunction::Floor)),
    spec(b"fmod", NativeFunction::Math(MathFunction::Fmod)),
    spec(b"frexp", NativeFunction::Math(MathFunction::Frexp)),
    spec(b"ldexp", NativeFunction::Math(MathFunction::Ldexp)),
    spec(b"log", NativeFunction::Math(MathFunction::Log)),
    spec(b"log10", NativeFunction::Math(MathFunction::Log10)),
    spec(b"max", NativeFunction::Math(MathFunction::Max)),
    spec(b"min", NativeFunction::Math(MathFunction::Min)),
    spec(b"modf", NativeFunction::Math(MathFunction::Modf)),
    spec(b"pow", NativeFunction::Math(MathFunction::Pow)),
    spec(b"rad", NativeFunction::Math(MathFunction::Rad)),
    spec(b"random", NativeFunction::Math(MathFunction::Random)),
    spec(
        b"randomseed",
        NativeFunction::Math(MathFunction::RandomSeed),
    ),
    spec(b"sin", NativeFunction::Math(MathFunction::Sin)),
    spec(b"sinh", NativeFunction::Math(MathFunction::Sinh)),
    spec(b"sqrt", NativeFunction::Math(MathFunction::Sqrt)),
    spec(b"tan", NativeFunction::Math(MathFunction::Tan)),
    spec(b"tanh", NativeFunction::Math(MathFunction::Tanh)),
];

pub const STRING_FUNCTIONS: &[FunctionSpec] = &[
    spec(b"byte", NativeFunction::String(StringFunction::Byte)),
    spec(b"char", NativeFunction::String(StringFunction::Char)),
    spec(b"find", NativeFunction::String(StringFunction::Find)),
    spec(b"format", NativeFunction::String(StringFunction::Format)),
    spec(b"len", NativeFunction::String(StringFunction::Len)),
    spec(b"lower", NativeFunction::String(StringFunction::Lower)),
    spec(b"rep", NativeFunction::String(StringFunction::Rep)),
    spec(b"reverse", NativeFunction::String(StringFunction::Reverse)),
    spec(b"sub", NativeFunction::String(StringFunction::Sub)),
    spec(b"upper", NativeFunction::String(StringFunction::Upper)),
];

pub const TABLE_FUNCTIONS: &[FunctionSpec] = &[
    spec(b"concat", NativeFunction::Table(TableFunction::Concat)),
    spec(b"insert", NativeFunction::Table(TableFunction::Insert)),
    spec(b"maxn", NativeFunction::Table(TableFunction::MaxN)),
    spec(b"remove", NativeFunction::Table(TableFunction::Remove)),
    spec(b"sort", NativeFunction::Table(TableFunction::Sort)),
];

pub const COROUTINE_FUNCTIONS: &[FunctionSpec] = &[
    spec(b"create", NativeFunction::CoroutineCreate),
    spec(b"resume", NativeFunction::CoroutineResume),
    spec(b"yield", NativeFunction::CoroutineYield),
    spec(b"status", NativeFunction::CoroutineStatus),
    spec(b"running", NativeFunction::CoroutineRunning),
    spec(b"wrap", NativeFunction::CoroutineWrap),
];

pub const IO_FUNCTIONS: &[FunctionSpec] = &[
    spec(b"open", NativeFunction::Io(IoFunction::Open)),
    spec(b"close", NativeFunction::Io(IoFunction::Close)),
    spec(b"type", NativeFunction::Io(IoFunction::Type)),
    spec(b"read", NativeFunction::Io(IoFunction::Read)),
    spec(b"write", NativeFunction::Io(IoFunction::Write)),
    spec(b"flush", NativeFunction::Io(IoFunction::Flush)),
];

pub const FILE_FUNCTIONS: &[FunctionSpec] = &[
    spec(b"read", NativeFunction::File(FileFunction::Read)),
    spec(b"write", NativeFunction::File(FileFunction::Write)),
    spec(b"close", NativeFunction::File(FileFunction::Close)),
    spec(b"flush", NativeFunction::File(FileFunction::Flush)),
    spec(b"seek", NativeFunction::File(FileFunction::Seek)),
];

pub const OS_FUNCTIONS: &[FunctionSpec] = &[
    spec(b"clock", NativeFunction::Os(OsFunction::Clock)),
    spec(b"date", NativeFunction::Os(OsFunction::Date)),
    spec(b"difftime", NativeFunction::Os(OsFunction::DiffTime)),
    spec(b"execute", NativeFunction::Os(OsFunction::Execute)),
    spec(b"getenv", NativeFunction::Os(OsFunction::GetEnv)),
    spec(b"remove", NativeFunction::Os(OsFunction::Remove)),
    spec(b"rename", NativeFunction::Os(OsFunction::Rename)),
    spec(b"time", NativeFunction::Os(OsFunction::Time)),
    spec(b"tmpname", NativeFunction::Os(OsFunction::TmpName)),
];

pub const DEBUG_FUNCTIONS: &[FunctionSpec] = &[
    spec(b"getinfo", NativeFunction::Debug(DebugFunction::GetInfo)),
    spec(
        b"getmetatable",
        NativeFunction::Debug(DebugFunction::GetMetatable),
    ),
    spec(
        b"setmetatable",
        NativeFunction::Debug(DebugFunction::SetMetatable),
    ),
    spec(
        b"getregistry",
        NativeFunction::Debug(DebugFunction::GetRegistry),
    ),
    spec(
        b"getupvalue",
        NativeFunction::Debug(DebugFunction::GetUpvalue),
    ),
    spec(
        b"setupvalue",
        NativeFunction::Debug(DebugFunction::SetUpvalue),
    ),
    spec(
        b"traceback",
        NativeFunction::Debug(DebugFunction::Traceback),
    ),
];

pub const PACKAGE_FUNCTIONS: &[FunctionSpec] = &[];

pub const LIBRARIES: &[LibrarySpec] = &[
    LibrarySpec {
        name: b"math",
        functions: MATH_FUNCTIONS,
    },
    LibrarySpec {
        name: b"string",
        functions: STRING_FUNCTIONS,
    },
    LibrarySpec {
        name: b"table",
        functions: TABLE_FUNCTIONS,
    },
    LibrarySpec {
        name: b"coroutine",
        functions: COROUTINE_FUNCTIONS,
    },
    LibrarySpec {
        name: b"io",
        functions: IO_FUNCTIONS,
    },
    LibrarySpec {
        name: b"os",
        functions: OS_FUNCTIONS,
    },
    LibrarySpec {
        name: b"package",
        functions: PACKAGE_FUNCTIONS,
    },
    LibrarySpec {
        name: b"debug",
        functions: DEBUG_FUNCTIONS,
    },
];

const fn spec(name: &'static [u8], function: NativeFunction) -> FunctionSpec {
    FunctionSpec { name, function }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostError {
    pub argument: Option<usize>,
    pub message: String,
}

impl HostError {
    pub fn argument(index: usize, message: impl Into<String>) -> Self {
        Self {
            argument: Some(index + 1),
            message: message.into(),
        }
    }

    pub fn message(message: impl Into<String>) -> Self {
        Self {
            argument: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(argument) = self.argument {
            write!(formatter, "bad argument #{argument} ({})", self.message)
        } else {
            formatter.write_str(&self.message)
        }
    }
}

impl std::error::Error for HostError {}

// ANCHOR: host_args
#[derive(Clone, Copy)]
pub struct Args<'a> {
    values: &'a [Value],
}

impl<'a> Args<'a> {
    pub fn new(values: &'a [Value]) -> Self {
        Self { values }
    }

    pub fn len(self) -> usize {
        self.values.len()
    }

    pub fn is_empty(self) -> bool {
        self.values.is_empty()
    }

    pub fn get(self, index: usize) -> Option<&'a Value> {
        self.values.get(index)
    }

    pub fn required(self, index: usize) -> Result<&'a Value, HostError> {
        self.get(index)
            .ok_or_else(|| HostError::argument(index, "value expected"))
    }

    pub fn number(self, index: usize) -> Result<f64, HostError> {
        match self.required(index)? {
            Value::Number(number) => Ok(*number),
            Value::String(bytes) => parse_number(bytes.as_bytes())
                .ok_or_else(|| HostError::argument(index, "number expected")),
            other => Err(HostError::argument(
                index,
                format!("number expected, got {}", other.type_name()),
            )),
        }
    }

    pub fn integer(self, index: usize) -> Result<i64, HostError> {
        let number = self.number(index)?;
        if !number.is_finite() || number < i64::MIN as f64 || number > i64::MAX as f64 {
            return Err(HostError::argument(index, "integer out of range"));
        }
        Ok(number.trunc() as i64)
    }

    pub fn optional_integer(self, index: usize, default: i64) -> Result<i64, HostError> {
        match self.get(index) {
            None | Some(Value::Nil) => Ok(default),
            Some(_) => self.integer(index),
        }
    }

    pub fn bytes(self, index: usize) -> Result<Vec<u8>, HostError> {
        match self.required(index)? {
            Value::String(bytes) => Ok(bytes.as_bytes().to_vec()),
            Value::Number(number) => Ok(number.to_string().into_bytes()),
            other => Err(HostError::argument(
                index,
                format!("string expected, got {}", other.type_name()),
            )),
        }
    }

    pub fn table(self, index: usize) -> Result<TableRef, HostError> {
        match self.required(index)? {
            Value::Table(table) => Ok(*table),
            other => Err(HostError::argument(
                index,
                format!("table expected, got {}", other.type_name()),
            )),
        }
    }

    pub fn tail(self, start: usize) -> &'a [Value] {
        self.values.get(start..).unwrap_or_default()
    }
}
// ANCHOR_END: host_args

pub fn bytes_value(bytes: impl AsRef<[u8]>) -> Value {
    Value::String(ByteString::new(bytes.as_ref()))
}

fn parse_number(bytes: &[u8]) -> Option<f64> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    (!text.is_empty()).then(|| text.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::{Args, BASE_FUNCTIONS, LIBRARIES};
    use lua_core::{ByteString, Value};

    #[test]
    fn adapters_apply_lua_number_and_byte_string_coercions() {
        let values = [
            Value::String(ByteString::new(&b" 42.5 "[..])),
            Value::Number(7.0),
        ];
        let args = Args::new(&values);
        assert_eq!(args.number(0).unwrap(), 42.5);
        assert_eq!(args.bytes(1).unwrap(), b"7");
    }

    #[test]
    fn argument_errors_are_one_based_and_classified() {
        let values = [Value::Boolean(true)];
        let error = Args::new(&values).number(0).unwrap_err();
        assert_eq!(error.argument, Some(1));
        assert!(error.to_string().starts_with("bad argument #1"));
    }

    #[test]
    fn catalog_has_unique_names_inside_each_namespace() {
        let mut names = BASE_FUNCTIONS
            .iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        let base_len = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), base_len);
        for library in LIBRARIES {
            let mut names = library
                .functions
                .iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>();
            let function_count = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), function_count);
        }
    }
}
