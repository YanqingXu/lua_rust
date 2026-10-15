//! Unique fixtures; no process-wide cwd/environment mutation.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct TempDirectory {
    path: Option<PathBuf>,
    parent: PathBuf,
}

impl TempDirectory {
    pub fn new(label: &str) -> Result<Self, String> {
        if label.is_empty()
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(
                "fixture label must contain only ASCII letters, digits, '-' or '_'".to_owned(),
            );
        }
        let parent = std::env::temp_dir()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path = parent.join(format!(
            "lua-rust-v18-{label}-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(|error| error.to_string())?;
        Ok(Self {
            path: Some(path),
            parent,
        })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        assert!(
            !name.is_empty()
                && Path::new(name).components().count() == 1
                && !matches!(name, "." | "..")
                && !Path::new(name).is_absolute(),
            "fixture filename must be one relative component"
        );
        self.path.as_ref().unwrap().join(name)
    }

    fn remove_checked(&self, path: &Path) -> Result<(), String> {
        let resolved = path.canonicalize().map_err(|error| error.to_string())?;
        if resolved.parent() != Some(self.parent.as_path())
            || resolved.file_name() != path.file_name()
            || !resolved
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("lua-rust-v18-"))
        {
            return Err("cleanup target escaped this unique fixture directory".to_owned());
        }
        fs::remove_dir_all(resolved).map_err(|error| error.to_string())
    }

    pub fn finish<T>(mut self, result: Result<T, String>) -> Result<T, String> {
        let path = self.path.take().unwrap();
        match (result, self.remove_checked(&path)) {
            (Err(primary), Err(cleanup)) => {
                Err(format!("{primary}; additionally cleanup: {cleanup}"))
            }
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(value), Ok(())) => Ok(value),
        }
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            if let Err(error) = self.remove_checked(path) {
                eprintln!("CLI fixture cleanup during early exit: {error}");
            }
        }
    }
}
