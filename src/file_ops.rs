use std::fs;
use std::io;
use std::path::Path;

pub(crate) fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    // ponytail: unsupported rename-replace filesystems fail; add a native fallback only if required.
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::replace_file;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temporary_directory() -> PathBuf {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        env::temp_dir().join(format!(
            "txbase-file-replace-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn replacement_updates_an_existing_file() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();
        let source = directory.join("source");
        let destination = directory.join("destination");
        fs::write(&source, b"new").unwrap();
        fs::write(&destination, b"old").unwrap();

        replace_file(&source, &destination).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"new");
        assert!(!source.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_replacement_preserves_the_existing_file() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();
        let source = directory.join("missing");
        let destination = directory.join("destination");
        fs::write(&destination, b"old").unwrap();

        assert!(replace_file(&source, &destination).is_err());

        assert_eq!(fs::read(&destination).unwrap(), b"old");
        fs::remove_dir_all(directory).unwrap();
    }
}
