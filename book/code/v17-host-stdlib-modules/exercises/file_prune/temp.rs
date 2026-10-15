use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const PREFIX: &str = "lua-rust-v17-file-prune-";

pub struct TempFiles {
    path: Option<PathBuf>,
    root: PathBuf,
}

impl TempFiles {
    pub fn new() -> Result<Self, String> {
        let raw_root = std::env::temp_dir();
        if !raw_root.is_absolute() {
            return Err("temporary root must be absolute".to_owned());
        }
        let root = fs::canonicalize(&raw_root).map_err(|error| error.to_string())?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path = raw_root.join(format!(
            "{PREFIX}{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(|error| error.to_string())?;
        Ok(Self {
            path: Some(path),
            root,
        })
    }

    pub fn file(&self, index: usize) -> PathBuf {
        self.path
            .as_ref()
            .unwrap()
            .join(format!("file-{index}.txt"))
    }

    fn remove_checked(&self, path: &Path) -> Result<(), String> {
        let resolved =
            fs::canonicalize(path).map_err(|error| format!("resolve cleanup target: {error}"))?;
        if resolved.parent() != Some(self.root.as_path())
            || !resolved
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(PREFIX))
        {
            return Err("cleanup target escaped the unique temporary directory".to_owned());
        }
        fs::remove_dir_all(&resolved)
            .map_err(|error| format!("cleanup {}: {error}", resolved.display()))
    }

    pub fn finish<T>(mut self, result: Result<T, String>) -> Result<T, String> {
        let path = self.path.take().unwrap();
        match (result, self.remove_checked(&path)) {
            (Err(primary), Err(cleanup)) => Err(format!("{primary}; additionally {cleanup}")),
            (Err(primary), Ok(())) => Err(primary),
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Ok(value), Ok(())) => Ok(value),
        }
    }
}

impl Drop for TempFiles {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            if let Err(error) = self.remove_checked(path) {
                eprintln!("temporary file cleanup during early exit: {error}");
            }
        }
    }
}

pub fn lua_quote(path: &Path) -> Result<String, String> {
    let text = path.to_str().ok_or("file path must be UTF-8")?;
    let bytes = text
        .bytes()
        .map(|byte| format!("\\{byte:03}"))
        .collect::<String>();
    Ok(format!("\"{bytes}\""))
}
