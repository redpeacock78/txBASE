use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

const WAL_MAGIC: [u8; 4] = *b"TXWL";
const WAL_HEADER_SIZE: usize = 16;
pub(crate) const MAX_WAL_RECORD_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Lsn(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TransactionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub lsn: Lsn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transaction {
    pub id: TransactionId,
    pub snapshot: Snapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    Snapshot,
}

#[derive(Debug)]
pub enum TransactionError {
    Io(String),
    Unsupported,
    Invalid(String),
}

impl std::fmt::Display for TransactionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "transaction I/O error: {message}"),
            Self::Unsupported => write!(formatter, "transaction operation is unsupported"),
            Self::Invalid(message) => write!(formatter, "invalid transaction: {message}"),
        }
    }
}

impl std::error::Error for TransactionError {}

impl From<io::Error> for TransactionError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

pub trait Wal {
    fn append(&mut self, record: &[u8]) -> Result<Lsn, TransactionError>;
    fn sync(&mut self) -> Result<(), TransactionError>;

    fn current_lsn(&self) -> Lsn {
        Lsn(0)
    }
}

#[derive(Debug, Default)]
pub struct MemoryWal {
    records: Vec<(Lsn, Vec<u8>)>,
    last_lsn: Option<Lsn>,
    synced: bool,
}

impl MemoryWal {
    pub fn records(&self) -> &[(Lsn, Vec<u8>)] {
        &self.records
    }

    pub fn is_synced(&self) -> bool {
        self.synced
    }
}

impl Wal for MemoryWal {
    fn append(&mut self, record: &[u8]) -> Result<Lsn, TransactionError> {
        validate_record_size(record)?;
        let lsn = next_lsn(self.last_lsn)?;
        self.records.push((lsn, record.to_vec()));
        self.last_lsn = Some(lsn);
        self.synced = false;
        Ok(lsn)
    }

    fn sync(&mut self) -> Result<(), TransactionError> {
        self.synced = true;
        Ok(())
    }

    fn current_lsn(&self) -> Lsn {
        self.last_lsn.unwrap_or(Lsn(0))
    }
}

pub struct FileWal {
    file: File,
    records: Vec<(Lsn, Vec<u8>)>,
    last_lsn: Option<Lsn>,
}

impl std::fmt::Debug for FileWal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileWal")
            .field("records", &self.records)
            .field("last_lsn", &self.last_lsn)
            .finish_non_exhaustive()
    }
}

impl FileWal {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TransactionError> {
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let mut records = Vec::new();
        let mut offset = 0usize;
        let mut expected_lsn = 0u64;

        while offset < bytes.len() {
            let remaining = bytes.len() - offset;
            if remaining < WAL_HEADER_SIZE {
                file.set_len(offset as u64)?;
                file.sync_all()?;
                break;
            }
            if bytes[offset..offset + WAL_MAGIC.len()] != WAL_MAGIC {
                return Err(TransactionError::Invalid(
                    "WAL record has an invalid magic".into(),
                ));
            }
            let length = u32::from_le_bytes(
                bytes[offset + 4..offset + 8]
                    .try_into()
                    .expect("WAL header length is fixed"),
            ) as usize;
            if length > MAX_WAL_RECORD_SIZE {
                return Err(TransactionError::Invalid(
                    "WAL record exceeds the configured size limit".into(),
                ));
            }
            let end = offset
                .checked_add(WAL_HEADER_SIZE)
                .and_then(|header_end| header_end.checked_add(length))
                .ok_or_else(|| TransactionError::Invalid("WAL record length overflows".into()))?;
            if end > bytes.len() {
                file.set_len(offset as u64)?;
                file.sync_all()?;
                break;
            }
            let lsn = u64::from_le_bytes(
                bytes[offset + 8..offset + 16]
                    .try_into()
                    .expect("WAL header LSN is fixed"),
            );
            if lsn != expected_lsn {
                return Err(TransactionError::Invalid(format!(
                    "WAL LSN {lsn} is not the expected {expected_lsn}"
                )));
            }
            records.push((Lsn(lsn), bytes[offset + WAL_HEADER_SIZE..end].to_vec()));
            expected_lsn = expected_lsn
                .checked_add(1)
                .ok_or_else(|| TransactionError::Invalid("WAL LSN overflows".into()))?;
            offset = end;
        }

        file.seek(SeekFrom::End(0))?;
        Ok(Self {
            file,
            last_lsn: records.last().map(|(lsn, _)| *lsn),
            records,
        })
    }

    pub fn records(&self) -> &[(Lsn, Vec<u8>)] {
        &self.records
    }

    pub fn clear(&mut self) -> Result<(), TransactionError> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.sync_all()?;
        self.records.clear();
        self.last_lsn = None;
        Ok(())
    }
}

impl Wal for FileWal {
    fn append(&mut self, record: &[u8]) -> Result<Lsn, TransactionError> {
        validate_record_size(record)?;
        let lsn = next_lsn(self.last_lsn)?;
        let length = u32::try_from(record.len())
            .map_err(|_| TransactionError::Invalid("WAL record length overflows".into()))?;
        let mut header = Vec::with_capacity(WAL_HEADER_SIZE);
        header.extend_from_slice(&WAL_MAGIC);
        header.extend_from_slice(&length.to_le_bytes());
        header.extend_from_slice(&lsn.0.to_le_bytes());
        self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&header)?;
        self.file.write_all(record)?;
        self.records.push((lsn, record.to_vec()));
        self.last_lsn = Some(lsn);
        Ok(lsn)
    }

    fn sync(&mut self) -> Result<(), TransactionError> {
        self.file.sync_all()?;
        Ok(())
    }

    fn current_lsn(&self) -> Lsn {
        self.last_lsn.unwrap_or(Lsn(0))
    }
}

fn validate_record_size(record: &[u8]) -> Result<(), TransactionError> {
    if record.len() > MAX_WAL_RECORD_SIZE {
        return Err(TransactionError::Invalid(
            "WAL record exceeds the configured size limit".into(),
        ));
    }
    Ok(())
}

fn next_lsn(last_lsn: Option<Lsn>) -> Result<Lsn, TransactionError> {
    match last_lsn {
        Some(Lsn(value)) => value
            .checked_add(1)
            .map(Lsn)
            .ok_or_else(|| TransactionError::Invalid("WAL LSN overflows".into())),
        None => Ok(Lsn(0)),
    }
}

pub trait Mvcc {
    fn current_lsn(&self) -> Lsn;

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lsn: self.current_lsn(),
        }
    }
}

pub trait TransactionEngine {
    fn begin(&mut self, isolation: IsolationLevel) -> Result<Transaction, TransactionError>;
    fn commit(&mut self, transaction: Transaction) -> Result<Lsn, TransactionError>;
    fn rollback(&mut self, transaction: Transaction) -> Result<(), TransactionError>;
}

pub struct TransactionManager<W: Wal> {
    wal: W,
    current_lsn: Lsn,
    next_transaction_id: u64,
    active: BTreeMap<TransactionId, Transaction>,
}

impl<W: Wal> TransactionManager<W> {
    pub fn new(wal: W) -> Self {
        let current_lsn = wal.current_lsn();
        Self {
            wal,
            current_lsn,
            next_transaction_id: 1,
            active: BTreeMap::new(),
        }
    }

    pub fn wal(&self) -> &W {
        &self.wal
    }

    pub fn into_wal(self) -> W {
        self.wal
    }

    fn finish(&mut self, transaction: Transaction, marker: u8) -> Result<Lsn, TransactionError> {
        if self.active.get(&transaction.id).copied() != Some(transaction) {
            return Err(TransactionError::Invalid(
                "transaction is not active".into(),
            ));
        }
        let lsn = self.wal.append(&transaction_record(marker, transaction))?;
        self.wal.sync()?;
        self.active.remove(&transaction.id);
        self.current_lsn = lsn;
        Ok(lsn)
    }
}

impl Default for TransactionManager<MemoryWal> {
    fn default() -> Self {
        Self::new(MemoryWal::default())
    }
}

pub type InMemoryTransactionEngine = TransactionManager<MemoryWal>;
pub type FileTransactionEngine = TransactionManager<FileWal>;

impl<W: Wal> Mvcc for TransactionManager<W> {
    fn current_lsn(&self) -> Lsn {
        self.current_lsn
    }
}

impl<W: Wal> TransactionEngine for TransactionManager<W> {
    fn begin(&mut self, isolation: IsolationLevel) -> Result<Transaction, TransactionError> {
        match isolation {
            IsolationLevel::Snapshot => {}
        }
        let id = TransactionId(self.next_transaction_id);
        self.next_transaction_id = self
            .next_transaction_id
            .checked_add(1)
            .ok_or_else(|| TransactionError::Invalid("transaction ID overflows".into()))?;
        let transaction = Transaction {
            id,
            snapshot: Snapshot {
                lsn: self.current_lsn,
            },
        };
        self.active.insert(id, transaction);
        Ok(transaction)
    }

    fn commit(&mut self, transaction: Transaction) -> Result<Lsn, TransactionError> {
        self.finish(transaction, b'C')
    }

    fn rollback(&mut self, transaction: Transaction) -> Result<(), TransactionError> {
        self.finish(transaction, b'R').map(|_| ())
    }
}

fn transaction_record(marker: u8, transaction: Transaction) -> Vec<u8> {
    let mut record = Vec::with_capacity(17);
    record.push(marker);
    record.extend_from_slice(&transaction.id.0.to_le_bytes());
    record.extend_from_slice(&transaction.snapshot.lsn.0.to_le_bytes());
    record
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod malformed_tests;
