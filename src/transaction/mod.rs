use std::collections::BTreeMap;
use std::io;

mod wal;

pub use wal::{FileWal, MemoryWal, WalInspection, WalRecordInfo};
pub(crate) use wal::{MAX_WAL_RECORD_SIZE, WAL_MAGIC};

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

    fn next_transaction_id(&self) -> u64 {
        1
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
        let next_transaction_id = wal.next_transaction_id();
        Self {
            wal,
            current_lsn,
            next_transaction_id,
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
