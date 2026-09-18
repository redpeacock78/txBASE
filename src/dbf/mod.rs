use crate::transaction::{FileWal, TransactionError, Wal};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

mod codec;
mod codepages;
mod lock;
mod memo;
#[cfg(test)]
mod tests;
mod wal;

use codec::{
    decode_field, decode_record_field, encode_character, encode_field, encode_null_flags,
    flag_is_set, hex, null_flag_layout, parse_fields, read_u16, read_u32, system_field_index, text,
    update_null_flags, value_text,
};
use lock::TableLock;
use memo::{
    binary_value, empty_memo_value, encode_memo_pointer, find_memo_path, is_sidecar_field,
    memo_index, sidecar_update, storage_value_without_sidecar,
};
#[cfg(test)]
use wal::{
    ByteDelta, DELTA_MAGIC, MEMO_SNAPSHOT_MAGIC, SNAPSHOT_MAGIC, apply_byte_delta, decode_snapshot,
};
use wal::{decode_wal_payload, delta_payload, memo_snapshot_payload, snapshot_payload};

const CLASSIC_HEADER_SIZE: usize = 32;
const CLASSIC_DESCRIPTOR_SIZE: usize = 32;
const LEVEL7_HEADER_SIZE: usize = 68;
const LEVEL7_DESCRIPTOR_SIZE: usize = 48;
const FIELD_TERMINATOR: u8 = 0x0d;
const EOF_MARKER: u8 = 0x1a;
const ACTIVE_RECORD: u8 = 0x20;
const DELETED_RECORD: u8 = 0x2a;
const DBT_BLOCK_SIZE: usize = 512;
const CURRENCY_SCALE: u64 = 10_000;
const MILLISECONDS_PER_DAY: u32 = 86_400_000;
const JULIAN_DAY_UNIX_EPOCH: i64 = 2_440_588;
#[derive(Debug)]
pub enum DbfError {
    Io(std::io::Error),
    Invalid(String),
}

impl Display for DbfError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Invalid(message) => write!(f, "invalid DBF: {message}"),
        }
    }
}

impl Error for DbfError {}

impl From<std::io::Error> for DbfError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbfHeader {
    pub version: u8,
    pub last_update: [u8; 3],
    pub record_count: u32,
    pub header_length: u16,
    pub record_length: u16,
    pub language_driver: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDescriptor {
    pub name: String,
    pub field_type: u8,
    pub length: u8,
    pub decimal_count: u8,
    pub flags: u8,
    pub offset: usize,
}

impl FieldDescriptor {
    fn is_system(&self) -> bool {
        self.flags & 0x01 != 0 || self.name.eq_ignore_ascii_case("_NULLFLAGS")
    }

    fn is_variable(&self) -> bool {
        matches!(self.field_type.to_ascii_uppercase(), b'Q' | b'V')
    }

    fn is_nullable(&self) -> bool {
        self.flags & 0x02 != 0
    }

    fn is_binary(&self) -> bool {
        matches!(
            self.field_type.to_ascii_uppercase(),
            b'Q' | b'G' | b'P' | b'W'
        ) || (self.field_type.eq_ignore_ascii_case(&b'B') && self.length != 8)
            || self.flags & 0x04 != 0
    }

    fn is_auto_increment(&self, version: u8) -> bool {
        self.field_type.eq_ignore_ascii_case(&b'+')
            || (version == 0x31
                && self.field_type.eq_ignore_ascii_case(&b'I')
                && self.flags & 0x0c == 0x0c)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NullFlagBits {
    varlength: Option<usize>,
    nullable: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbfRecord {
    pub number: usize,
    pub deleted: bool,
    pub values: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoFormat {
    Dbase3,
    Dbase4,
    FoxPro,
}

impl MemoFormat {
    fn tag(self) -> u8 {
        match self {
            Self::Dbase3 => 0,
            Self::Dbase4 => 1,
            Self::FoxPro => 2,
        }
    }

    fn from_tag(tag: u8) -> Result<Self, DbfError> {
        match tag {
            0 => Ok(Self::Dbase3),
            1 => Ok(Self::Dbase4),
            2 => Ok(Self::FoxPro),
            _ => Err(DbfError::Invalid(format!(
                "unknown memo sidecar format tag {tag}"
            ))),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Dbase3 | Self::Dbase4 => "dbt",
            Self::FoxPro => "fpt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoFile {
    bytes: Vec<u8>,
    block_size: usize,
    format: MemoFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoSnapshot {
    format: MemoFormat,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MemoUpdate {
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PersistedState {
    path: PathBuf,
    dbf: Vec<u8>,
    memo: Option<Vec<u8>>,
}

type MemoUpdates = BTreeMap<(usize, String), MemoUpdate>;
type PreparedStorage = (Map<String, Value>, MemoUpdates);

#[derive(Debug, Clone, PartialEq)]
pub struct DbfTable {
    pub header: DbfHeader,
    pub fields: Vec<FieldDescriptor>,
    records: Vec<DbfRecord>,
    stored_values: Vec<Map<String, Value>>,
    bytes: Vec<u8>,
    memo: Option<MemoFile>,
    memo_updates: MemoUpdates,
    source: Option<PersistedState>,
}

fn memo_format_for_version(version: u8) -> Option<MemoFormat> {
    match version {
        0x83 => Some(MemoFormat::Dbase3),
        0x8b => Some(MemoFormat::Dbase4),
        0x30 | 0x31 | 0x32 | 0xf5 => Some(MemoFormat::FoxPro),
        _ => None,
    }
}

impl DbfTable {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, DbfError> {
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        Self::recover_wal(path)?;
        let dbf = fs::read(path)?;
        let mut table = Self::from_bytes(&dbf)?;
        if table.has_sidecar_fields() {
            if let Some(memo_path) = find_memo_path(path) {
                let memo = MemoFile::open(&memo_path, table.header.version)?;
                table.resolve_memos(&memo)?;
                table.memo = Some(memo);
            }
        }
        table.source = Some(PersistedState {
            path: path.to_path_buf(),
            dbf,
            memo: table.memo.as_ref().map(|memo| memo.bytes.clone()),
        });
        Ok(table)
    }

    fn has_sidecar_fields(&self) -> bool {
        self.fields.iter().any(is_sidecar_field)
    }

    fn resolve_memos(&mut self, memo: &MemoFile) -> Result<(), DbfError> {
        let fields = self
            .fields
            .iter()
            .filter(|field| is_sidecar_field(field))
            .cloned()
            .collect::<Vec<_>>();
        for index in 0..self.records.len() {
            if self.records[index].deleted {
                continue;
            }
            let record_offset = self.record_offset(index)?;
            for field in &fields {
                if self.records[index]
                    .values
                    .get(&field.name)
                    .is_some_and(Value::is_null)
                {
                    continue;
                }
                let start = record_offset + field.offset;
                let end = start + usize::from(field.length);
                let Some(block) = memo_index(&self.bytes[start..end], memo.format)? else {
                    continue;
                };
                let Some(data) = memo.read(block)? else {
                    continue;
                };
                let value = if field.is_binary() {
                    Value::String(hex(&data))
                } else {
                    Value::String(text(&data, self.header.language_driver))
                };
                self.records[index].values.insert(field.name.clone(), value);
            }
        }
        Ok(())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DbfError> {
        if bytes.len() < CLASSIC_HEADER_SIZE {
            return Err(DbfError::Invalid("header is truncated".into()));
        }

        let header = DbfHeader {
            version: bytes[0],
            last_update: [bytes[1], bytes[2], bytes[3]],
            record_count: read_u32(bytes, 4)?,
            header_length: read_u16(bytes, 8)?,
            record_length: read_u16(bytes, 10)?,
            language_driver: bytes[29],
        };
        let header_length = usize::from(header.header_length);
        let record_length = usize::from(header.record_length);

        if header_length < CLASSIC_HEADER_SIZE + 1 || header_length > bytes.len() {
            return Err(DbfError::Invalid(format!(
                "header length {} is outside the file",
                header.header_length
            )));
        }
        if record_length == 0 {
            return Err(DbfError::Invalid("record length is zero".into()));
        }

        let (descriptor_start, descriptor_size) = if header.version & 0x07 == 4 {
            (LEVEL7_HEADER_SIZE, LEVEL7_DESCRIPTOR_SIZE)
        } else {
            (CLASSIC_HEADER_SIZE, CLASSIC_DESCRIPTOR_SIZE)
        };
        if descriptor_start >= header_length {
            return Err(DbfError::Invalid("field descriptor area is missing".into()));
        }

        let terminator = (descriptor_start..header_length)
            .step_by(descriptor_size)
            .find(|offset| bytes[*offset] == FIELD_TERMINATOR)
            .ok_or_else(|| DbfError::Invalid("field descriptor terminator is missing".into()))?;

        let fields = parse_fields(bytes, descriptor_start, terminator, descriptor_size)?;
        let flag_layout = null_flag_layout(&fields);
        let system_field = system_field_index(&fields);
        let foxpro_table = matches!(header.version, 0x30..=0x32);
        let variable_fields = header.version == 0x32;
        let memo_format = memo_format_for_version(header.version);
        let expected_record_length = fields
            .iter()
            .map(|field| field.length as usize)
            .sum::<usize>()
            .checked_add(1)
            .ok_or_else(|| DbfError::Invalid("record length overflows usize".into()))?;
        if expected_record_length != record_length {
            return Err(DbfError::Invalid(format!(
                "record length {} does not match fields ({expected_record_length})",
                header.record_length
            )));
        }

        let record_start = header_length;
        let record_bytes = usize::try_from(header.record_count)
            .ok()
            .and_then(|count| count.checked_mul(record_length))
            .ok_or_else(|| DbfError::Invalid("record area overflows usize".into()))?;
        let record_end = record_start
            .checked_add(record_bytes)
            .ok_or_else(|| DbfError::Invalid("record area overflows usize".into()))?;
        if record_end > bytes.len() {
            return Err(DbfError::Invalid("record area is truncated".into()));
        }

        let record_count = usize::try_from(header.record_count)
            .map_err(|_| DbfError::Invalid("record count overflows usize".into()))?;
        let mut records = Vec::with_capacity(record_count);
        let mut stored_values = Vec::with_capacity(record_count);
        for number in 1..=record_count {
            let start = record_start + (number - 1) * record_length;
            let record = &bytes[start..start + record_length];
            let deleted = match record[0] {
                ACTIVE_RECORD => false,
                DELETED_RECORD => true,
                marker => {
                    return Err(DbfError::Invalid(format!(
                        "record {number} has unknown deletion marker 0x{marker:02x}"
                    )));
                }
            };

            let null_flags = system_field.and_then(|index| {
                let field = &fields[index];
                record.get(field.offset..field.offset + usize::from(field.length))
            });
            let mut values = Map::new();
            for (field_index, field) in fields.iter().enumerate() {
                if field.is_system() {
                    continue;
                }
                let start = field.offset;
                let end = start + field.length as usize;
                let is_null = foxpro_table
                    && flag_layout[field_index]
                        .and_then(|bits| bits.nullable)
                        .is_some_and(|bit| null_flags.is_some_and(|flags| flag_is_set(flags, bit)));
                let value = if is_null {
                    Value::Null
                } else if variable_fields && field.is_variable() {
                    decode_record_field(
                        field,
                        &record[start..end],
                        header.language_driver,
                        null_flags,
                        flag_layout[field_index],
                    )
                } else if field.field_type.eq_ignore_ascii_case(&b'C') && field.is_binary() {
                    Value::String(hex(&record[start..end]))
                } else {
                    decode_field(
                        field.field_type,
                        &record[start..end],
                        header.language_driver,
                        memo_format,
                    )
                };
                values.insert(field.name.clone(), value);
            }
            stored_values.push(values.clone());
            records.push(DbfRecord {
                number,
                deleted,
                values,
            });
        }

        Ok(Self {
            header,
            fields,
            records,
            stored_values,
            bytes: bytes.to_vec(),
            memo: None,
            memo_updates: BTreeMap::new(),
            source: None,
        })
    }

    pub fn records(&self) -> &[DbfRecord] {
        &self.records
    }

    pub fn active_records(&self) -> impl Iterator<Item = &DbfRecord> {
        self.records.iter().filter(|record| !record.deleted)
    }

    pub fn active_json(&self) -> Vec<Value> {
        self.active_records()
            .map(|record| Value::Object(record.values.clone()))
            .collect()
    }

    pub fn active_record(&self, number: usize) -> Option<&DbfRecord> {
        number
            .checked_sub(1)
            .and_then(|index| self.records.get(index).filter(|record| !record.deleted))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<(), DbfError> {
        if !self.memo_updates.is_empty() {
            return Err(DbfError::Invalid(
                "memo updates require save_with_wal".into(),
            ));
        }
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        Self::recover_wal(path)?;
        self.ensure_source_current(path)?;
        save_bytes_to(path, &self.bytes, "txbase.tmp")
    }

    fn ensure_source_current(&self, path: &Path) -> Result<(), DbfError> {
        let Some(source) = &self.source else {
            return Ok(());
        };
        if source.path != path {
            return Ok(());
        }
        let current_dbf = fs::read(path)?;
        if current_dbf.as_slice() != source.dbf.as_slice() {
            return Err(DbfError::Invalid(
                "DBF changed since the table was loaded".into(),
            ));
        }
        if let Some(expected_memo) = &source.memo {
            let Some(memo_path) = find_memo_path(path) else {
                return Err(DbfError::Invalid(
                    "memo sidecar changed since the table was loaded".into(),
                ));
            };
            let current_memo = fs::read(memo_path)?;
            if current_memo.as_slice() != expected_memo.as_slice() {
                return Err(DbfError::Invalid(
                    "memo sidecar changed since the table was loaded".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn save_with_wal(&mut self, path: impl AsRef<Path>) -> Result<(), DbfError> {
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        Self::recover_wal(path)?;
        self.ensure_source_current(path)?;
        let mut prepared = self.clone();
        let memo_snapshot = prepared.apply_memo_updates(path)?;
        let wal_path = path.with_extension("txbase.wal");
        let mut wal = FileWal::open(&wal_path).map_err(transaction_error)?;
        let full_payload = match &memo_snapshot {
            Some(memo) => memo_snapshot_payload(&prepared.bytes, memo)?,
            None => snapshot_payload(&prepared.bytes),
        };
        let payload = delta_payload(&prepared, path, memo_snapshot.as_ref(), full_payload.len())?
            .unwrap_or(full_payload);
        wal.append(&payload).map_err(transaction_error)?;
        wal.sync().map_err(transaction_error)?;
        if let Some(memo) = &memo_snapshot {
            let memo_path = find_memo_path(path)
                .unwrap_or_else(|| path.with_extension(memo.format.extension()));
            save_bytes_to(&memo_path, &memo.bytes, "txbase.memo.tmp")?;
        }
        save_bytes_to(path, &prepared.bytes, "txbase.tmp")?;
        if wal.clear().is_ok() {
            drop(wal);
            let _ = fs::remove_file(wal_path);
        }
        prepared.source = Some(PersistedState {
            path: path.to_path_buf(),
            dbf: prepared.bytes.clone(),
            memo: prepared.memo.as_ref().map(|memo| memo.bytes.clone()),
        });
        *self = prepared;
        Ok(())
    }

    fn recover_wal(path: &Path) -> Result<(), DbfError> {
        let wal_path = path.with_extension("txbase.wal");
        if !wal_path.exists() {
            return Ok(());
        }
        let mut wal = FileWal::open(&wal_path).map_err(transaction_error)?;
        let snapshot =
            wal.records().iter().rev().find_map(|(_, payload)| {
                match decode_wal_payload(path, payload) {
                    Ok(Some(snapshot)) => Some(Ok(snapshot)),
                    Ok(None) => None,
                    Err(error) => Some(Err(error)),
                }
            });
        let Some(snapshot) = snapshot else {
            return Ok(());
        };
        let snapshot = snapshot?;
        Self::from_bytes(&snapshot.dbf)?;
        if let Some(memo) = &snapshot.memo {
            let memo_path = find_memo_path(path)
                .unwrap_or_else(|| path.with_extension(memo.format.extension()));
            save_bytes_to(&memo_path, &memo.bytes, "txbase.memo.tmp")?;
        }
        save_bytes_to(path, &snapshot.dbf, "txbase.tmp")?;
        if wal.clear().is_ok() {
            drop(wal);
            let _ = fs::remove_file(wal_path);
        }
        Ok(())
    }

    pub fn insert_record(&mut self, values: Map<String, Value>) -> Result<usize, DbfError> {
        let mut values = self.normalize_values(&values)?;
        let mut auto_increment_updates = Vec::new();
        for (field_index, field) in self.fields.iter().enumerate() {
            if !field.is_auto_increment(self.header.version) {
                continue;
            }
            if !values[&field.name].is_null() {
                return Err(DbfError::Invalid(format!(
                    "auto-increment field {} is read-only",
                    field.name
                )));
            }
            let Some((next, following, descriptor_offset, signed)) =
                self.next_auto_increment(field_index, field)?
            else {
                continue;
            };
            values.insert(field.name.clone(), Value::Number(next.into()));
            let following = if signed {
                i32::try_from(following)
                    .map_err(|_| {
                        DbfError::Invalid(format!(
                            "auto-increment field {} is exhausted",
                            field.name
                        ))
                    })?
                    .to_le_bytes()
            } else {
                u32::try_from(following)
                    .map_err(|_| {
                        DbfError::Invalid(format!(
                            "auto-increment field {} is exhausted",
                            field.name
                        ))
                    })?
                    .to_le_bytes()
            };
            auto_increment_updates.push((descriptor_offset, following));
        }
        let number = self
            .records
            .len()
            .checked_add(1)
            .ok_or_else(|| DbfError::Invalid("record count overflows usize".into()))?;
        let new_count = u32::try_from(number)
            .map_err(|_| DbfError::Invalid("record count exceeds DBF limit".into()))?;
        let mut storage_values = values.clone();
        let mut memo_updates = BTreeMap::new();
        if let Some(memo_format) = self.memo.as_ref().map(|memo| memo.format) {
            for field in self.fields.iter().filter(|field| is_sidecar_field(field)) {
                let value = values.get(&field.name).unwrap_or(&Value::Null);
                storage_values.insert(field.name.clone(), empty_memo_value(field));
                if let Some(update) = sidecar_update(value, field, memo_format)? {
                    memo_updates.insert((number - 1, field.name.clone()), update);
                }
            }
        } else {
            for field in self.fields.iter().filter(|field| is_sidecar_field(field)) {
                let value = values.get(&field.name).unwrap_or(&Value::Null);
                storage_values.insert(
                    field.name.clone(),
                    storage_value_without_sidecar(value, field)?,
                );
            }
        }
        let encoded = self.encode_record(&storage_values)?;
        let record_end = self.record_end()?;
        if self.bytes.len() < record_end {
            return Err(DbfError::Invalid(
                "record area is shorter than parsed data".into(),
            ));
        }
        let suffix = &self.bytes[record_end..];
        if !suffix.is_empty() && suffix != [EOF_MARKER] {
            return Err(DbfError::Invalid(
                "cannot mutate a DBF with trailing sidecar data".into(),
            ));
        }

        let mut bytes = self.bytes[..record_end].to_vec();
        bytes.extend_from_slice(&encoded);
        bytes.push(EOF_MARKER);
        write_record_count(&mut bytes, new_count)?;
        for (descriptor_offset, next) in auto_increment_updates {
            let target = bytes
                .get_mut(descriptor_offset..descriptor_offset + 4)
                .ok_or_else(|| {
                    DbfError::Invalid("auto-increment descriptor is truncated".into())
                })?;
            target.copy_from_slice(&next);
        }

        self.bytes = bytes;
        self.header.record_count = new_count;
        self.records.push(DbfRecord {
            number,
            deleted: false,
            values: values.clone(),
        });
        self.stored_values.push(storage_values);
        self.memo_updates.extend(memo_updates);
        Ok(number)
    }

    pub fn replace_record(
        &mut self,
        number: usize,
        values: Map<String, Value>,
    ) -> Result<(), DbfError> {
        let index = self.active_index(number)?;
        let changed_fields = values.keys().cloned().collect::<BTreeSet<_>>();
        let mut values = self.normalize_values(&values)?;
        self.preserve_auto_increment_fields(index, &mut values, &changed_fields)?;
        let (storage_values, memo_updates) = self.prepare_existing_storage(index, &values, None)?;
        self.write_existing_record(index, &values, &storage_values)?;
        self.memo_updates = memo_updates;
        Ok(())
    }

    pub fn patch_record(
        &mut self,
        number: usize,
        patch: Map<String, Value>,
    ) -> Result<(), DbfError> {
        let index = self.active_index(number)?;
        let (values, changed_fields) = expand_update(&self.records[index].values, patch)?;
        let mut values = self.normalize_values(&values)?;
        self.preserve_auto_increment_fields(index, &mut values, &changed_fields)?;
        let (storage_values, memo_updates) =
            self.prepare_existing_storage(index, &values, Some(&changed_fields))?;
        self.write_existing_record(index, &values, &storage_values)?;
        self.memo_updates = memo_updates;
        Ok(())
    }

    pub fn delete_record(&mut self, number: usize) -> Result<(), DbfError> {
        let index = self.active_index(number)?;
        let offset = self.record_offset(index)?;
        let mut bytes = self.bytes.clone();
        let marker = bytes
            .get_mut(offset)
            .ok_or_else(|| DbfError::Invalid("record area is truncated".into()))?;
        *marker = DELETED_RECORD;
        self.bytes = bytes;
        self.records[index].deleted = true;
        Ok(())
    }

    fn normalize_values(
        &self,
        values: &Map<String, Value>,
    ) -> Result<Map<String, Value>, DbfError> {
        for field in values.keys() {
            if !self
                .fields
                .iter()
                .any(|descriptor| !descriptor.is_system() && descriptor.name == *field)
            {
                return Err(DbfError::Invalid(format!("unknown field {field}")));
            }
        }
        Ok(self
            .fields
            .iter()
            .filter(|field| !field.is_system())
            .map(|field| {
                (
                    field.name.clone(),
                    values.get(&field.name).cloned().unwrap_or(Value::Null),
                )
            })
            .collect())
    }

    fn encode_record(&self, storage_values: &Map<String, Value>) -> Result<Vec<u8>, DbfError> {
        let mut record = Vec::with_capacity(usize::from(self.header.record_length));
        record.push(ACTIVE_RECORD);
        let null_flags = encode_null_flags(&self.fields, storage_values)?;
        for field in &self.fields {
            if field.is_system() {
                let flags = null_flags.as_deref().ok_or_else(|| {
                    DbfError::Invalid("system field requires _NullFlags bytes".into())
                })?;
                if flags.len() != usize::from(field.length) {
                    return Err(DbfError::Invalid(
                        "_NullFlags field length is inconsistent".into(),
                    ));
                }
                record.extend_from_slice(flags);
            } else {
                record.extend(encode_field(
                    field,
                    storage_values.get(&field.name).unwrap_or(&Value::Null),
                    self.header.language_driver,
                )?);
            }
        }
        if record.len() != usize::from(self.header.record_length) {
            return Err(DbfError::Invalid(
                "encoded record length is incorrect".into(),
            ));
        }
        Ok(record)
    }

    fn active_index(&self, number: usize) -> Result<usize, DbfError> {
        let index = number
            .checked_sub(1)
            .ok_or_else(|| DbfError::Invalid("record id must be a positive integer".into()))?;
        match self.records.get(index) {
            Some(record) if !record.deleted => Ok(index),
            _ => Err(DbfError::Invalid("record not found".into())),
        }
    }

    fn record_end(&self) -> Result<usize, DbfError> {
        let record_bytes = self
            .records
            .len()
            .checked_mul(usize::from(self.header.record_length))
            .ok_or_else(|| DbfError::Invalid("record area overflows usize".into()))?;
        usize::from(self.header.header_length)
            .checked_add(record_bytes)
            .ok_or_else(|| DbfError::Invalid("record area overflows usize".into()))
    }

    fn record_offset(&self, index: usize) -> Result<usize, DbfError> {
        let offset = index
            .checked_mul(usize::from(self.header.record_length))
            .and_then(|offset| offset.checked_add(usize::from(self.header.header_length)))
            .ok_or_else(|| DbfError::Invalid("record offset overflows usize".into()))?;
        let end = offset
            .checked_add(usize::from(self.header.record_length))
            .ok_or_else(|| DbfError::Invalid("record offset overflows usize".into()))?;
        if end > self.bytes.len() {
            return Err(DbfError::Invalid("record area is truncated".into()));
        }
        Ok(offset)
    }

    fn next_auto_increment(
        &self,
        field_index: usize,
        field: &FieldDescriptor,
    ) -> Result<Option<(i64, i64, usize, bool)>, DbfError> {
        if self.header.version & 0x07 == 4 {
            let descriptor_offset =
                LEVEL7_HEADER_SIZE
                    .checked_add(field_index.checked_mul(LEVEL7_DESCRIPTOR_SIZE).ok_or_else(
                        || DbfError::Invalid("field descriptor offset overflows".into()),
                    )?)
                    .ok_or_else(|| DbfError::Invalid("field descriptor offset overflows".into()))?;
            let next_offset = descriptor_offset
                .checked_add(40)
                .ok_or_else(|| DbfError::Invalid("auto-increment offset overflows".into()))?;
            let next_end = next_offset
                .checked_add(4)
                .ok_or_else(|| DbfError::Invalid("auto-increment offset overflows".into()))?;
            if next_end > usize::from(self.header.header_length) {
                return Err(DbfError::Invalid(
                    "auto-increment descriptor is outside the header".into(),
                ));
            }
            let next = read_u32(&self.bytes, next_offset)?;
            return Ok(Some((
                i64::from(next),
                i64::from(next) + 1,
                next_offset,
                false,
            )));
        }

        if self.header.version != 0x31 {
            return Ok(None);
        }
        if field.length != 4 || !field.field_type.eq_ignore_ascii_case(&b'I') {
            return Err(DbfError::Invalid(format!(
                "auto-increment field {} must be a four-byte integer",
                field.name
            )));
        }
        let descriptor_offset = CLASSIC_HEADER_SIZE
            .checked_add(
                field_index
                    .checked_mul(CLASSIC_DESCRIPTOR_SIZE)
                    .ok_or_else(|| DbfError::Invalid("field descriptor offset overflows".into()))?,
            )
            .ok_or_else(|| DbfError::Invalid("field descriptor offset overflows".into()))?;
        let next_offset = descriptor_offset
            .checked_add(19)
            .ok_or_else(|| DbfError::Invalid("auto-increment offset overflows".into()))?;
        let step_offset = descriptor_offset
            .checked_add(23)
            .ok_or_else(|| DbfError::Invalid("auto-increment offset overflows".into()))?;
        let end = step_offset
            .checked_add(1)
            .ok_or_else(|| DbfError::Invalid("auto-increment offset overflows".into()))?;
        if end > usize::from(self.header.header_length) {
            return Err(DbfError::Invalid(
                "auto-increment descriptor is outside the header".into(),
            ));
        }
        let next = i64::from(i32::from_le_bytes(
            read_u32(&self.bytes, next_offset)?.to_le_bytes(),
        ));
        let step = self.bytes[step_offset];
        if step == 0 {
            return Err(DbfError::Invalid(format!(
                "auto-increment field {} has a zero step",
                field.name
            )));
        }
        let following = next.checked_add(i64::from(step)).ok_or_else(|| {
            DbfError::Invalid(format!("auto-increment field {} is exhausted", field.name))
        })?;
        Ok(Some((following, following, next_offset, true)))
    }

    fn preserve_auto_increment_fields(
        &self,
        index: usize,
        values: &mut Map<String, Value>,
        changed_fields: &BTreeSet<String>,
    ) -> Result<(), DbfError> {
        for field in self
            .fields
            .iter()
            .filter(|field| field.is_auto_increment(self.header.version))
        {
            let current = &self.records[index].values[&field.name];
            if changed_fields.contains(&field.name) && values[&field.name] != *current {
                return Err(DbfError::Invalid(format!(
                    "auto-increment field {} is read-only",
                    field.name
                )));
            }
            values.insert(field.name.clone(), current.clone());
        }
        Ok(())
    }

    fn write_existing_record(
        &mut self,
        index: usize,
        values: &Map<String, Value>,
        storage_values: &Map<String, Value>,
    ) -> Result<(), DbfError> {
        let offset = self.record_offset(index)?;
        let mut bytes = self.bytes.clone();
        let mut changed_fields = BTreeSet::new();
        for field in &self.fields {
            if field.is_system() {
                continue;
            }
            if values.get(&field.name) == self.records[index].values.get(&field.name) {
                continue;
            }
            changed_fields.insert(field.name.clone());
            let encoded = encode_field(
                field,
                storage_values.get(&field.name).unwrap_or(&Value::Null),
                self.header.language_driver,
            )?;
            let start = offset
                .checked_add(field.offset)
                .ok_or_else(|| DbfError::Invalid("field offset overflows usize".into()))?;
            let end = start
                .checked_add(usize::from(field.length))
                .ok_or_else(|| DbfError::Invalid("field range overflows usize".into()))?;
            bytes
                .get_mut(start..end)
                .ok_or_else(|| DbfError::Invalid("stored field is truncated".into()))?
                .copy_from_slice(&encoded);
        }
        update_null_flags(&mut bytes, offset, &self.fields, values, &changed_fields)?;
        self.bytes = bytes;
        self.records[index].values = values.clone();
        self.stored_values[index] = storage_values.clone();
        Ok(())
    }

    fn prepare_existing_storage(
        &self,
        index: usize,
        values: &Map<String, Value>,
        changed_fields: Option<&BTreeSet<String>>,
    ) -> Result<PreparedStorage, DbfError> {
        let mut storage_values = values.clone();
        let mut memo_updates = self.memo_updates.clone();
        for field in self.fields.iter().filter(|field| is_sidecar_field(field)) {
            let changed = changed_fields.is_none_or(|fields| fields.contains(&field.name));
            let key = (index, field.name.clone());
            if !changed || values.get(&field.name) == self.records[index].values.get(&field.name) {
                if self.memo.is_some() {
                    storage_values.insert(
                        field.name.clone(),
                        self.stored_values[index][&field.name].clone(),
                    );
                }
                continue;
            }

            if self.memo.is_none() {
                storage_values.insert(
                    field.name.clone(),
                    storage_value_without_sidecar(&values[&field.name], field)?,
                );
                continue;
            }

            if self.memo.is_some() {
                if !field.is_binary() {
                    let text = value_text(&values[&field.name], field)?;
                    storage_values.insert(field.name.clone(), empty_memo_value(field));
                    if text.is_empty() {
                        memo_updates.remove(&key);
                    } else {
                        memo_updates.insert(key, MemoUpdate::Text(text));
                    }
                } else {
                    let memo_format = self
                        .memo
                        .as_ref()
                        .map(|memo| memo.format)
                        .expect("memo presence checked above");
                    let update = sidecar_update(&values[&field.name], field, memo_format)?;
                    storage_values.insert(field.name.clone(), empty_memo_value(field));
                    if let Some(update) = update {
                        memo_updates.insert(key, update);
                    } else {
                        memo_updates.remove(&key);
                    }
                }
            }
        }
        Ok((storage_values, memo_updates))
    }

    fn apply_memo_updates(&mut self, path: &Path) -> Result<Option<MemoSnapshot>, DbfError> {
        if self.memo_updates.is_empty() {
            return Ok(None);
        }
        if find_memo_path(path).is_none() {
            return Err(DbfError::Invalid(
                "memo sidecar is missing for pending memo updates".into(),
            ));
        }
        let mut memo = self
            .memo
            .clone()
            .ok_or_else(|| DbfError::Invalid("memo sidecar is not loaded".into()))?;

        for ((index, field_name), value) in self.memo_updates.clone() {
            let Some(record) = self.records.get(index) else {
                return Err(DbfError::Invalid(
                    "memo update record is out of range".into(),
                ));
            };
            if record.deleted {
                continue;
            }
            let field = self
                .fields
                .iter()
                .find(|field| field.name == field_name && is_sidecar_field(field))
                .cloned()
                .ok_or_else(|| {
                    DbfError::Invalid(format!("sidecar field {field_name} not found"))
                })?;
            let pointer = match value {
                MemoUpdate::Text(value) => {
                    if value.is_empty() {
                        encode_memo_pointer(&field, 0, memo.format)?
                    } else {
                        let bytes = encode_character(
                            &Value::String(value),
                            &field,
                            self.header.language_driver,
                        )?;
                        let block = memo.append_text(&bytes)?;
                        encode_memo_pointer(&field, block, memo.format)?
                    }
                }
                MemoUpdate::Binary(bytes) => {
                    if bytes.is_empty() {
                        encode_memo_pointer(&field, 0, memo.format)?
                    } else {
                        let block = memo.append_binary(&bytes)?;
                        encode_memo_pointer(&field, block, memo.format)?
                    }
                }
            };
            let record_offset = self.record_offset(index)?;
            let start = record_offset
                .checked_add(field.offset)
                .ok_or_else(|| DbfError::Invalid("memo pointer offset overflows usize".into()))?;
            let end = start
                .checked_add(usize::from(field.length))
                .ok_or_else(|| DbfError::Invalid("memo pointer end overflows usize".into()))?;
            let target = self
                .bytes
                .get_mut(start..end)
                .ok_or_else(|| DbfError::Invalid("memo pointer area is truncated".into()))?;
            target.copy_from_slice(&pointer);
            self.stored_values[index].insert(
                field.name,
                decode_field(
                    field.field_type,
                    target,
                    self.header.language_driver,
                    Some(memo.format),
                ),
            );
        }

        self.memo = Some(memo.clone());
        self.memo_updates.clear();
        Ok(Some(MemoSnapshot {
            format: memo.format,
            bytes: memo.bytes,
        }))
    }
}

fn expand_update(
    current: &Map<String, Value>,
    update: Map<String, Value>,
) -> Result<(Map<String, Value>, BTreeSet<String>), DbfError> {
    let has_operator = update.keys().any(|key| key.starts_with('$'));
    if !has_operator {
        let changed_fields = update.keys().cloned().collect();
        let mut values = current.clone();
        values.extend(update);
        return Ok((values, changed_fields));
    }
    if update.keys().any(|key| !key.starts_with('$')) {
        return Err(DbfError::Invalid(
            "update cannot mix operators and fields".into(),
        ));
    }

    let mut values = current.clone();
    let mut changed_fields = BTreeSet::new();
    for (operator, operand) in update {
        match operator.as_str() {
            "$set" | "$unset" | "$inc" => {}
            _ => {
                return Err(DbfError::Invalid(format!(
                    "unsupported update operator {operator}"
                )));
            }
        }
        let fields = operand
            .as_object()
            .ok_or_else(|| DbfError::Invalid(format!("{operator} requires an object")))?;
        for field in fields.keys() {
            if !changed_fields.insert(field.clone()) {
                return Err(DbfError::Invalid(format!(
                    "field {field} appears in multiple update operators"
                )));
            }
        }
        match operator.as_str() {
            "$set" => values.extend(fields.clone()),
            "$unset" => {
                for field in fields.keys() {
                    values.insert(field.clone(), Value::Null);
                }
            }
            "$inc" => {
                for (field, increment) in fields {
                    let value = increment_value(values.get(field), increment, field)?;
                    values.insert(field.clone(), value);
                }
            }
            _ => unreachable!("update operator was validated above"),
        }
    }
    Ok((values, changed_fields))
}

fn increment_value(
    current: Option<&Value>,
    increment: &Value,
    field: &str,
) -> Result<Value, DbfError> {
    let Some(Value::Number(current)) = current else {
        return Err(DbfError::Invalid(format!(
            "$inc requires a numeric value in field {field}"
        )));
    };
    let Value::Number(increment) = increment else {
        return Err(DbfError::Invalid(format!(
            "$inc value for {field} must be a JSON number"
        )));
    };
    if let (Some(current), Some(increment)) = (current.as_i64(), increment.as_i64()) {
        let value = current
            .checked_add(increment)
            .ok_or_else(|| DbfError::Invalid(format!("$inc overflows integer field {field}")))?;
        return Ok(Value::Number(value.into()));
    }
    let value = current
        .as_f64()
        .and_then(|current| increment.as_f64().map(|increment| current + increment))
        .and_then(Number::from_f64)
        .ok_or_else(|| DbfError::Invalid(format!("$inc result for {field} is not finite")))?;
    Ok(Value::Number(value))
}

fn save_bytes_to(path: &Path, bytes: &[u8], temporary_extension: &str) -> Result<(), DbfError> {
    let temporary_path = path.with_extension(temporary_extension);
    let mut file = fs::File::create(&temporary_path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temporary_path, path)?;
    sync_parent_directory(path)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), DbfError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<(), DbfError> {
    Ok(())
}

fn transaction_error(error: TransactionError) -> DbfError {
    DbfError::Invalid(format!("WAL error: {error}"))
}

fn write_record_count(bytes: &mut [u8], count: u32) -> Result<(), DbfError> {
    let header = bytes
        .get_mut(4..8)
        .ok_or_else(|| DbfError::Invalid("header is truncated".into()))?;
    header.copy_from_slice(&count.to_le_bytes());
    Ok(())
}
