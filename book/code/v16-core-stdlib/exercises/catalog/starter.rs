use crate::model::{InstallError, Installed};
use lua_stdlib::{FunctionSpec, LibrarySpec};

// ANCHOR: catalog_install_starter
pub fn install(
    base: &[FunctionSpec],
    libraries: &[LibrarySpec],
) -> Result<Installed, InstallError> {
    // TODO：统一按每个 FunctionSpec 的 name / function 安装，不能遗漏或改写 ID。
    // 先运行教学 preflight；base 进 global，每个 library 用各自的 table。
    crate::faults::omit_sqrt(base, libraries)
}
// ANCHOR_END: catalog_install_starter
