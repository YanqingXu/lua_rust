//! Locate the bytecode tool; all spawning/draining remains in the shared process runner.
use std::path::{Path, PathBuf};

#[allow(dead_code)] // Some teaching targets use only the example locator.
pub fn sibling_of_lua_app(lua_app: &Path) -> Result<PathBuf, String> {
    let profile = lua_app
        .parent()
        .ok_or("lua_app path has no profile directory")?;
    let name = if cfg!(windows) {
        "lua_bytecode.exe"
    } else {
        "lua_bytecode"
    };
    let binary = profile.join(name);
    if binary.is_file() {
        Ok(binary)
    } else {
        Err(format!("missing bytecode sibling {}; first run: cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace --bins",binary.display()))
    }
}

#[allow(dead_code)] // Integration targets start from env!(CARGO_BIN_EXE_lua_app).
pub fn example_binary() -> Result<PathBuf, String> {
    let app = crate::process::example_binary().map_err(|error| format!("{error}; bytecode tools require cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace --bins"))?;
    sibling_of_lua_app(&app)
}
