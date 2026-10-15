use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

pub struct TempDirectory {
    root: PathBuf,
    parent: PathBuf,
    active: bool,
}

impl TempDirectory {
    pub fn new() -> Result<Self, String> {
        let parent = std::env::temp_dir()
            .canonicalize()
            .map_err(|error| format!("canonical temp root: {error}"))?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let root = parent.join(format!(
            "lua-v17-file-drain-{}-{stamp}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).map_err(|error| format!("create fixture: {error}"))?;
        Ok(Self {
            root,
            parent,
            active: true,
        })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn remove_checked(&self) -> Result<(), String> {
        let resolved = self
            .root
            .canonicalize()
            .map_err(|error| format!("canonical fixture: {error}"))?;
        if resolved.parent() != Some(self.parent.as_path())
            || resolved.file_name() != self.root.file_name()
            || !resolved.starts_with(&self.parent)
        {
            return Err("fixture cleanup escaped its original temporary parent".into());
        }
        fs::remove_dir_all(&resolved).map_err(|error| format!("cleanup fixture: {error}"))
    }

    pub fn finish<T>(mut self, result: Result<T, String>) -> Result<T, String> {
        let cleanup = self.remove_checked();
        self.active = false;
        match (result, cleanup) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(cleanup)) => Err(format!("{error}; additionally {cleanup}")),
        }
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        if self.active {
            if let Err(error) = self.remove_checked() {
                eprintln!("file_drain fallback cleanup: {error}");
            }
        }
    }
}

pub fn lua_path(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .ok_or_else(|| "fixture path is not UTF-8".to_string())?;
    Ok(text.replace('\\', "/").replace('\'', "\\'"))
}
