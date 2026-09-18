#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Lsn(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    Unsupported,
    Invalid(String),
}

pub trait Wal {
    fn append(&mut self, record: &[u8]) -> Result<Lsn, TransactionError>;
    fn sync(&mut self) -> Result<(), TransactionError>;
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
