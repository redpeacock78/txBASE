use super::{Lsn, TransactionError, Wal};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub(crate) const WAL_MAGIC: [u8; 4] = *b"TXWL";
const WAL_HEADER_SIZE: usize = 16;
pub(crate) const MAX_WAL_RECORD_SIZE: usize = 16 * 1024 * 1024;

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

    fn next_transaction_id(&self) -> u64 {
        next_transaction_id_after_records(&self.records)
    }
}

pub struct FileWal {
    file: File,
    records: Vec<(Lsn, Vec<u8>)>,
    last_lsn: Option<Lsn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalRecordInfo {
    pub lsn: Lsn,
    pub length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalInspection {
    pub file_bytes: usize,
    pub valid_bytes: usize,
    pub truncated_tail: bool,
    pub records: Vec<WalRecordInfo>,
}

struct ParsedWal {
    records: Vec<(Lsn, Vec<u8>)>,
    valid_bytes: usize,
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
        let ParsedWal {
            records,
            valid_bytes,
        } = parse_wal(&bytes)?;
        if valid_bytes < bytes.len() {
            file.set_len(valid_bytes as u64)?;
            file.sync_all()?;
        }

        file.seek(SeekFrom::End(0))?;
        Ok(Self {
            file,
            last_lsn: records.last().map(|(lsn, _)| *lsn),
            records,
        })
    }

    pub fn inspect(path: impl AsRef<Path>) -> Result<WalInspection, TransactionError> {
        let bytes = std::fs::read(path)?;
        let ParsedWal {
            records,
            valid_bytes,
        } = parse_wal(&bytes)?;
        Ok(WalInspection {
            file_bytes: bytes.len(),
            valid_bytes,
            truncated_tail: valid_bytes < bytes.len(),
            records: records
                .into_iter()
                .map(|(lsn, payload)| WalRecordInfo {
                    lsn,
                    length: payload.len(),
                })
                .collect(),
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

fn parse_wal(bytes: &[u8]) -> Result<ParsedWal, TransactionError> {
    let mut records = Vec::new();
    let mut offset = 0usize;
    let mut expected_lsn = 0u64;

    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        if remaining < WAL_HEADER_SIZE {
            return Ok(ParsedWal {
                records,
                valid_bytes: offset,
            });
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
            return Ok(ParsedWal {
                records,
                valid_bytes: offset,
            });
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

    Ok(ParsedWal {
        records,
        valid_bytes: bytes.len(),
    })
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

    fn next_transaction_id(&self) -> u64 {
        next_transaction_id_after_records(&self.records)
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

fn next_transaction_id_after_records(records: &[(Lsn, Vec<u8>)]) -> u64 {
    records
        .iter()
        .filter_map(|(_, record)| transaction_id_from_record(record))
        .max()
        .map_or(1, |id| id.saturating_add(1))
}

fn transaction_id_from_record(record: &[u8]) -> Option<u64> {
    (record.len() == 17 && matches!(record[0], b'C' | b'R'))
        .then(|| u64::from_le_bytes(record[1..9].try_into().expect("transaction ID is fixed")))
}
