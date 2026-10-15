use std::path::{Path, PathBuf};

#[allow(dead_code)] // Integration targets use env!(CARGO_BIN_EXE_runner_fixture).
pub fn example_binary() -> Result<PathBuf, String> {
    let example = std::env::current_exe().map_err(|error| error.to_string())?;
    let profile = example
        .parent()
        .and_then(Path::parent)
        .ok_or("example has no profile directory")?;
    let binary = profile.join(if cfg!(windows) {
        "runner_fixture.exe"
    } else {
        "runner_fixture"
    });
    if binary.is_file() {
        Ok(binary)
    } else {
        Err(format!("missing fixture sibling {}; first run: cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace --bins",binary.display()))
    }
}
