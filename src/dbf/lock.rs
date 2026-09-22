#[cfg(not(target_arch = "wasm32"))]
use fs2::FileExt;
#[cfg(not(target_arch = "wasm32"))]
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct TableLock {
    _file: File,
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct TableLock;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct TableReadLock {
    _file: File,
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct TableReadLock;

impl TableLock {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn acquire(path: &Path) -> io::Result<Self> {
        let lock_path = path.with_extension("txbase.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        FileExt::lock_exclusive(&file)?;
        Ok(Self { _file: file })
    }
}

#[cfg(target_arch = "wasm32")]
impl TableLock {
    pub(crate) fn acquire(_path: &Path) -> io::Result<Self> {
        // WASM has no shared POSIX file namespace. The host owns serialization.
        Ok(Self)
    }
}

impl TableReadLock {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn acquire(path: &Path) -> io::Result<Self> {
        let lock_path = path.with_extension("txbase.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        FileExt::lock_shared(&file)?;
        Ok(Self { _file: file })
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn acquire(_path: &Path) -> io::Result<Self> {
        Ok(Self)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::TableLock;
    use std::fs;

    #[test]
    fn acquires_and_releases_a_table_lock() {
        let path =
            std::env::temp_dir().join(format!("txbase-lock-test-{}.dbf", std::process::id()));
        let lock_path = path.with_extension("txbase.lock");
        let _ = fs::remove_file(&lock_path);
        {
            let _lock = TableLock::acquire(&path).unwrap();
            assert!(lock_path.exists());
        }
        fs::remove_file(lock_path).unwrap();
    }
}
