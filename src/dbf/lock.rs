use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

pub(super) struct TableLock {
    _file: File,
}

impl TableLock {
    pub(super) fn acquire(path: &Path) -> io::Result<Self> {
        let lock_path = path.with_extension("txbase.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        file.lock_exclusive()?;
        Ok(Self { _file: file })
    }
}
