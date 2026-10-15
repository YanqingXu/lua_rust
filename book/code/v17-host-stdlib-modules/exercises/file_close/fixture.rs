//! Unique temporary directory with explicit cleanup and a non-panicking Drop fallback.
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct Fixture {
    root: PathBuf,
    parent: PathBuf,
    active: bool,
}

impl Fixture {
    pub fn new() -> Self {
        let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(parent.is_absolute());
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = parent.join(format!(
            "lua-rust-v17-close-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self {
            root,
            parent,
            active: true,
        }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub fn open(&self, name: &str) -> File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(self.path(name))
            .unwrap()
    }

    fn remove_checked(&self) -> std::io::Result<()> {
        let resolved = fs::canonicalize(&self.root)?;
        if resolved.parent() != Some(self.parent.as_path())
            || resolved.file_name() != self.root.file_name()
            || !resolved
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("lua-rust-v17-close-"))
        {
            return Err(std::io::Error::other(
                "cleanup target escaped this fixture's unique directory",
            ));
        }
        fs::remove_dir_all(resolved)
    }

    pub fn finish(mut self) {
        self.active = false;
        self.remove_checked()
            .expect("explicit fixture cleanup must succeed");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.active {
            if let Err(error) = self.remove_checked() {
                eprintln!("file-close fixture cleanup during early exit: {error}");
            }
        }
    }
}

// ANCHOR: file_close_exclusive_probe
pub fn exclusive_available(path: &Path) -> Option<bool> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        match OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(path)
        {
            Ok(file) => {
                drop(file);
                Some(true)
            }
            Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => Some(false),
            Err(error) => panic!("unexpected exclusive reopen error: {error}"),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None // Unlink success would not prove that an existing handle is closed.
    }
}
// ANCHOR_END: file_close_exclusive_probe
