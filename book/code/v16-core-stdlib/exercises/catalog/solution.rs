use crate::model::{preflight, InstallError, Installed};
use lua_stdlib::{FunctionSpec, LibrarySpec};

// ANCHOR: catalog_install_solution
pub fn install(
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
            installed.bind(table, spec)?;
        }
    }
    Ok(installed)
}
// ANCHOR_END: catalog_install_solution
