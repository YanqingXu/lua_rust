//! 用真实 Heap / FunctionSpec 安装函数绑定；不实现 builtin 函数体或完整 VM 初始化。
use lua_core::{ByteString, Heap, HeapError, NativeFunction, TableRef, Value};
use lua_stdlib::{FunctionSpec, LibrarySpec};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub type BindingKey = (Vec<u8>, Vec<u8>);
pub type Bindings = BTreeMap<BindingKey, NativeFunction>;
pub type Installer = fn(&[FunctionSpec], &[LibrarySpec]) -> Result<Installed, InstallError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InstallError {
    Duplicate { scope: Vec<u8>, name: Vec<u8> },
    Heap(HeapError),
    InvalidBinding,
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate { scope, name } => write!(
                f,
                "duplicate {}.{}",
                String::from_utf8_lossy(scope),
                String::from_utf8_lossy(name)
            ),
            Self::Heap(error) => error.fmt(f),
            Self::InvalidBinding => f.write_str("expected named native-function binding"),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<HeapError> for InstallError {
    fn from(error: HeapError) -> Self {
        Self::Heap(error)
    }
}

fn claim(names: &mut BTreeSet<Vec<u8>>, scope: &[u8], name: &[u8]) -> Result<(), InstallError> {
    if !names.insert(name.to_vec()) {
        return Err(InstallError::Duplicate {
            scope: scope.to_vec(),
            name: name.to_vec(),
        });
    }
    Ok(())
}

// ANCHOR: catalog_preflight
/// 仅教学 preflight：真实 VM 初始化没有调用此检查。
pub fn preflight(base: &[FunctionSpec], libraries: &[LibrarySpec]) -> Result<(), InstallError> {
    let mut global_names = BTreeSet::new();
    for spec in base {
        claim(&mut global_names, b"_G", spec.name)?;
    }
    for library in libraries {
        claim(&mut global_names, b"_G", library.name)?;
        let mut local_names = BTreeSet::new();
        for spec in library.functions {
            claim(&mut local_names, library.name, spec.name)?;
        }
    }
    Ok(())
}
// ANCHOR_END: catalog_preflight

fn key(name: &[u8]) -> Value {
    Value::String(ByteString::new(name))
}

pub struct Installed {
    heap: Heap,
    global: TableRef,
}

impl Installed {
    pub fn empty() -> Result<Self, InstallError> {
        let mut heap = Heap::new();
        let global = heap.allocate_table()?;
        Ok(Self { heap, global })
    }

    pub fn global(&self) -> TableRef {
        self.global
    }

    pub fn library(&mut self, name: &[u8]) -> Result<TableRef, InstallError> {
        let table = self.heap.allocate_table()?;
        self.heap
            .table_set(self.global, key(name), Value::Table(table))?;
        Ok(table)
    }

    pub fn bind(&mut self, table: TableRef, spec: FunctionSpec) -> Result<(), InstallError> {
        self.heap
            .table_set(table, key(spec.name), Value::NativeFunction(spec.function))?;
        Ok(())
    }

    pub fn lookup(&self, scope: &[u8], name: &[u8]) -> Result<Value, InstallError> {
        let table = if scope.is_empty() {
            self.global
        } else {
            match self.heap.table_get(self.global, &key(scope))? {
                Value::Table(table) => table,
                Value::Nil => return Ok(Value::Nil),
                _ => return Err(InstallError::InvalidBinding),
            }
        };
        Ok(self.heap.table_get(table, &key(name))?)
    }

    fn entries(&self, table: TableRef) -> Result<Vec<(Value, Value)>, InstallError> {
        let mut cursor = Value::Nil;
        let mut entries = Vec::new();
        while let Some((name, value)) = self.heap.table_next_pair(table, &cursor)? {
            cursor = name.clone();
            entries.push((name, value));
        }
        Ok(entries)
    }

    pub fn snapshot(&self) -> Result<Bindings, InstallError> {
        let mut result = Bindings::new();
        for (name, value) in self.entries(self.global)? {
            let Value::String(name) = name else {
                return Err(InstallError::InvalidBinding);
            };
            match value {
                Value::NativeFunction(function) => {
                    result.insert((Vec::new(), name.as_bytes().to_vec()), function);
                }
                Value::Table(table) => {
                    for (member, value) in self.entries(table)? {
                        let (Value::String(member), Value::NativeFunction(function)) =
                            (member, value)
                        else {
                            return Err(InstallError::InvalidBinding);
                        };
                        result.insert(
                            (name.as_bytes().to_vec(), member.as_bytes().to_vec()),
                            function,
                        );
                    }
                }
                _ => return Err(InstallError::InvalidBinding),
            }
        }
        Ok(result)
    }
}

pub fn declared_bindings(
    base: &[FunctionSpec],
    libraries: &[LibrarySpec],
) -> Result<Bindings, InstallError> {
    preflight(base, libraries)?;
    let mut result = Bindings::new();
    for spec in base {
        result.insert((Vec::new(), spec.name.to_vec()), spec.function);
    }
    for library in libraries {
        for spec in library.functions {
            result.insert((library.name.to_vec(), spec.name.to_vec()), spec.function);
        }
    }
    Ok(result)
}
