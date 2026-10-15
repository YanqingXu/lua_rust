use crate::model::{preflight, InstallError, Installed};
use lua_core::{MathFunction, NativeFunction};
use lua_stdlib::{FunctionSpec, LibrarySpec};

// ANCHOR: catalog_missing_fault
/// 固定漏安装反例：catalog 已有 sqrt，安装路径仍遗漏它。
pub fn omit_sqrt(
    base: &[FunctionSpec],
    libraries: &[LibrarySpec],
) -> Result<Installed, InstallError> {
    preflight(base, libraries)?;
    let mut installed = Installed::empty()?;
    for &spec in base {
        installed.bind(installed.global(), spec)?;
    }
    for library in libraries {
        let table = installed.library(library.name)?;
        for &spec in library.functions {
            if library.name == b"math" && spec.name == b"sqrt" {
                continue;
            }
            installed.bind(table, spec)?;
        }
    }
    Ok(installed)
}
// ANCHOR_END: catalog_missing_fault

pub fn wrong_sqrt_id(
    base: &[FunctionSpec],
    libraries: &[LibrarySpec],
) -> Result<Installed, InstallError> {
    preflight(base, libraries)?;
    let mut installed = Installed::empty()?;
    for &spec in base {
        installed.bind(installed.global(), spec)?;
    }
    for library in libraries {
        let table = installed.library(library.name)?;
        for &spec in library.functions {
            let spec = if library.name == b"math" && spec.name == b"sqrt" {
                FunctionSpec {
                    name: spec.name,
                    function: NativeFunction::Math(MathFunction::Abs),
                }
            } else {
                spec
            };
            installed.bind(table, spec)?;
        }
    }
    Ok(installed)
}
