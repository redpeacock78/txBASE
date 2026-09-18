use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub trait Storage {
    fn read_range(&mut self, offset: u64, length: usize) -> io::Result<Vec<u8>>;
    fn write_range(&mut self, offset: u64, bytes: &[u8]) -> io::Result<()>;
    fn sync(&mut self) -> io::Result<()>;
}

pub struct FileStorage {
    file: File,
}

impl FileStorage {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            file: OpenOptions::new().read(true).write(true).open(path)?,
        })
    }
}

impl Storage for FileStorage {
    fn read_range(&mut self, offset: u64, length: usize) -> io::Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; length];
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    fn write_range(&mut self, offset: u64, bytes: &[u8]) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(bytes)
    }

    fn sync(&mut self) -> io::Result<()> {
        self.file.sync_all()
    }
}

#[derive(Default)]
pub struct MemoryStorage {
    bytes: Vec<u8>,
}

impl MemoryStorage {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Storage for MemoryStorage {
    fn read_range(&mut self, offset: u64, length: usize) -> io::Result<Vec<u8>> {
        let start = usize::try_from(offset)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "offset overflows usize"))?;
        let end = start
            .checked_add(length)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "range overflows usize"))?;
        self.bytes
            .get(start..end)
            .map(ToOwned::to_owned)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "range is outside storage"))
    }

    fn write_range(&mut self, offset: u64, bytes: &[u8]) -> io::Result<()> {
        let start = usize::try_from(offset)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "offset overflows usize"))?;
        let end = start
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "range overflows usize"))?;
        if end > self.bytes.len() {
            self.bytes.resize(end, 0);
        }
        self.bytes[start..end].copy_from_slice(bytes);
        Ok(())
    }

    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn memory_storage_round_trips_and_expands_ranges() {
        let mut storage = MemoryStorage::new(b"abc".to_vec());
        assert_eq!(storage.read_range(1, 2).unwrap(), b"bc");

        storage.write_range(3, b"de").unwrap();
        storage.sync().unwrap();
        assert_eq!(storage.into_inner(), b"abcde");
    }

    #[test]
    fn memory_storage_rejects_invalid_ranges() {
        let mut storage = MemoryStorage::new(b"abc".to_vec());
        assert_eq!(
            storage.read_range(2, 2).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(
            storage.read_range(u64::MAX, 1).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            storage.write_range(u64::MAX, b"x").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(storage.into_inner(), b"abc");
    }

    #[test]
    fn file_storage_syncs_range_changes() {
        let path =
            std::env::temp_dir().join(format!("txbase-storage-test-{}.bin", std::process::id()));
        let _ = fs::remove_file(&path);
        fs::write(&path, b"abc").unwrap();

        let mut storage = FileStorage::open(&path).unwrap();
        assert_eq!(storage.read_range(0, 3).unwrap(), b"abc");
        storage.write_range(1, b"XY").unwrap();
        storage.sync().unwrap();
        drop(storage);

        assert_eq!(fs::read(&path).unwrap(), b"aXY");
        fs::remove_file(path).unwrap();
    }
}
