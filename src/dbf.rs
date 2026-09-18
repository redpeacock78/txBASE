use crate::transaction::{FileWal, TransactionError, Wal};
use serde_json::{Map, Number, Value};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io::Write;
use std::path::Path;

const CLASSIC_HEADER_SIZE: usize = 32;
const CLASSIC_DESCRIPTOR_SIZE: usize = 32;
const LEVEL7_HEADER_SIZE: usize = 68;
const LEVEL7_DESCRIPTOR_SIZE: usize = 48;
const FIELD_TERMINATOR: u8 = 0x0d;
const EOF_MARKER: u8 = 0x1a;
const SNAPSHOT_MAGIC: &[u8; 4] = b"TXDB";
const ACTIVE_RECORD: u8 = 0x20;
const DELETED_RECORD: u8 = 0x2a;
const DBT_BLOCK_SIZE: usize = 512;

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
    pub offset: usize,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoFile {
    bytes: Vec<u8>,
    block_size: usize,
    format: MemoFormat,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbfTable {
    pub header: DbfHeader,
    pub fields: Vec<FieldDescriptor>,
    records: Vec<DbfRecord>,
    stored_values: Vec<Map<String, Value>>,
    bytes: Vec<u8>,
    memo: Option<MemoFile>,
}

impl DbfTable {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, DbfError> {
        let path = path.as_ref();
        Self::recover_wal(path)?;
        let mut table = Self::from_bytes(&fs::read(path)?)?;
        if table.has_memo_fields() {
            if let Some(memo_path) = find_memo_path(path) {
                let memo = MemoFile::open(&memo_path, table.header.version)?;
                table.resolve_memos(&memo)?;
                table.memo = Some(memo);
            }
        }
        Ok(table)
    }

    fn has_memo_fields(&self) -> bool {
        self.fields
            .iter()
            .any(|field| field.field_type.eq_ignore_ascii_case(&b'M'))
    }

    fn resolve_memos(&mut self, memo: &MemoFile) -> Result<(), DbfError> {
        let fields = self
            .fields
            .iter()
            .filter(|field| field.field_type.eq_ignore_ascii_case(&b'M'))
            .cloned()
            .collect::<Vec<_>>();
        for index in 0..self.records.len() {
            if self.records[index].deleted {
                continue;
            }
            let record_offset = self.record_offset(index)?;
            for field in &fields {
                let start = record_offset + field.offset;
                let end = start + usize::from(field.length);
                let Some(block) = memo_index(&self.bytes[start..end])? else {
                    continue;
                };
                let Some(data) = memo.read(block)? else {
                    continue;
                };
                self.records[index].values.insert(
                    field.name.clone(),
                    Value::String(text(&data, self.header.language_driver)),
                );
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

            let mut values = Map::new();
            for field in &fields {
                let start = field.offset;
                let end = start + field.length as usize;
                values.insert(
                    field.name.clone(),
                    decode_field(
                        field.field_type,
                        &record[start..end],
                        header.language_driver,
                    ),
                );
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
        let path = path.as_ref();
        let temporary_path = path.with_extension("txbase.tmp");
        let mut file = fs::File::create(&temporary_path)?;
        file.write_all(&self.bytes)?;
        file.sync_all()?;
        fs::rename(temporary_path, path)?;
        Ok(())
    }

    pub fn save_with_wal(&self, path: impl AsRef<Path>) -> Result<(), DbfError> {
        let path = path.as_ref();
        let wal_path = path.with_extension("txbase.wal");
        let mut wal = FileWal::open(&wal_path).map_err(transaction_error)?;
        let mut payload = Vec::with_capacity(SNAPSHOT_MAGIC.len() + self.bytes.len());
        payload.extend_from_slice(SNAPSHOT_MAGIC);
        payload.extend_from_slice(&self.bytes);
        wal.append(&payload).map_err(transaction_error)?;
        wal.sync().map_err(transaction_error)?;
        self.save_to(path)?;
        if wal.clear().is_ok() {
            drop(wal);
            let _ = fs::remove_file(wal_path);
        }
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
                payload.strip_prefix(SNAPSHOT_MAGIC).map(ToOwned::to_owned)
            });
        let Some(snapshot) = snapshot else {
            return Ok(());
        };
        let recovered = Self::from_bytes(&snapshot)?;
        recovered.save_to(path)?;
        if wal.clear().is_ok() {
            drop(wal);
            let _ = fs::remove_file(wal_path);
        }
        Ok(())
    }

    pub fn insert_record(&mut self, values: Map<String, Value>) -> Result<usize, DbfError> {
        let values = self.normalize_values(&values)?;
        if self.memo.is_some()
            && self.fields.iter().any(|field| {
                is_memo_field(field.field_type)
                    && values
                        .get(&field.name)
                        .is_some_and(|value| !value.is_null() && value != "")
            })
        {
            return Err(DbfError::Invalid(
                "memo field writes require a memo sidecar writer".into(),
            ));
        }
        let encoded = self.encode_record(&values)?;
        let number = self
            .records
            .len()
            .checked_add(1)
            .ok_or_else(|| DbfError::Invalid("record count overflows usize".into()))?;
        let new_count = u32::try_from(number)
            .map_err(|_| DbfError::Invalid("record count exceeds DBF limit".into()))?;
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

        self.bytes = bytes;
        self.header.record_count = new_count;
        self.records.push(DbfRecord {
            number,
            deleted: false,
            values: values.clone(),
        });
        self.stored_values.push(values);
        Ok(number)
    }

    pub fn replace_record(
        &mut self,
        number: usize,
        values: Map<String, Value>,
    ) -> Result<(), DbfError> {
        let index = self.active_index(number)?;
        let values = self.normalize_values(&values)?;
        let mut storage_values = values.clone();
        if self.memo.is_some() {
            for field in &self.fields {
                if is_memo_field(field.field_type) {
                    if values.get(&field.name) != self.records[index].values.get(&field.name) {
                        return Err(DbfError::Invalid(format!(
                            "memo field {} cannot be changed without a memo sidecar writer",
                            field.name
                        )));
                    }
                    storage_values.insert(
                        field.name.clone(),
                        self.stored_values[index][&field.name].clone(),
                    );
                }
            }
        }
        self.write_existing_record(index, &values, &storage_values)
    }

    pub fn patch_record(
        &mut self,
        number: usize,
        patch: Map<String, Value>,
    ) -> Result<(), DbfError> {
        let index = self.active_index(number)?;
        let mut values = self.records[index].values.clone();
        let changed_fields = patch.keys().cloned().collect::<BTreeSet<_>>();
        for (field, value) in patch {
            values.insert(field, value);
        }
        let values = self.normalize_values(&values)?;
        let mut storage_values = values.clone();
        for field in &self.fields {
            if is_memo_field(field.field_type)
                && (!changed_fields.contains(&field.name)
                    || values.get(&field.name) == self.records[index].values.get(&field.name))
            {
                storage_values.insert(
                    field.name.clone(),
                    self.stored_values[index][&field.name].clone(),
                );
            } else if is_memo_field(field.field_type)
                && self.memo.is_some()
                && values.get(&field.name) != self.records[index].values.get(&field.name)
            {
                return Err(DbfError::Invalid(format!(
                    "memo field {} cannot be changed without a memo sidecar writer",
                    field.name
                )));
            }
        }
        self.write_existing_record(index, &values, &storage_values)
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
                .any(|descriptor| descriptor.name == *field)
            {
                return Err(DbfError::Invalid(format!("unknown field {field}")));
            }
        }
        Ok(self
            .fields
            .iter()
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
        for field in &self.fields {
            record.extend(encode_field(
                field,
                storage_values.get(&field.name).unwrap_or(&Value::Null),
                self.header.language_driver,
            )?);
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

    fn write_existing_record(
        &mut self,
        index: usize,
        values: &Map<String, Value>,
        storage_values: &Map<String, Value>,
    ) -> Result<(), DbfError> {
        let encoded = self.encode_record(storage_values)?;
        let offset = self.record_offset(index)?;
        let end = offset + encoded.len();
        let mut bytes = self.bytes.clone();
        bytes[offset..end].copy_from_slice(&encoded);
        self.bytes = bytes;
        self.records[index].values = values.clone();
        self.stored_values[index] = storage_values.clone();
        Ok(())
    }
}

impl MemoFile {
    fn open(path: &Path, dbf_version: u8) -> Result<Self, DbfError> {
        let bytes = fs::read(path)?;
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("fpt"))
        {
            let block_size = bytes
                .get(6..8)
                .map(|header| usize::from(u16::from_be_bytes([header[0], header[1]])))
                .ok_or_else(|| DbfError::Invalid("FPT header is truncated".into()))?;
            if block_size < 8 || bytes.len() < block_size {
                return Err(DbfError::Invalid(
                    "FPT block size or header is invalid".into(),
                ));
            }
            Ok(Self {
                bytes,
                block_size,
                format: MemoFormat::FoxPro,
            })
        } else {
            if bytes.len() < DBT_BLOCK_SIZE {
                return Err(DbfError::Invalid("DBT header is truncated".into()));
            }
            Ok(Self {
                bytes,
                block_size: DBT_BLOCK_SIZE,
                format: if dbf_version == 0x83 {
                    MemoFormat::Dbase3
                } else {
                    MemoFormat::Dbase4
                },
            })
        }
    }

    fn read(&self, block: u32) -> Result<Option<Vec<u8>>, DbfError> {
        if block == 0 {
            return Ok(None);
        }
        let start = usize::try_from(block)
            .ok()
            .and_then(|block| block.checked_mul(self.block_size))
            .ok_or_else(|| DbfError::Invalid("memo block offset overflows usize".into()))?;
        if start >= self.bytes.len() {
            return Err(DbfError::Invalid(format!(
                "memo block {block} is outside the sidecar"
            )));
        }
        match self.format {
            MemoFormat::Dbase3 => {
                let data = &self.bytes[start..];
                let end = data
                    .iter()
                    .position(|byte| *byte == EOF_MARKER)
                    .unwrap_or(data.len());
                Ok(Some(data[..end].to_vec()))
            }
            MemoFormat::Dbase4 => {
                let header_end = start
                    .checked_add(8)
                    .ok_or_else(|| DbfError::Invalid("memo header overflows usize".into()))?;
                let header = self
                    .bytes
                    .get(start..header_end)
                    .ok_or_else(|| DbfError::Invalid("memo block header is truncated".into()))?;
                let length = usize::try_from(u32::from_le_bytes([
                    header[4], header[5], header[6], header[7],
                ]))
                .map_err(|_| DbfError::Invalid("memo length overflows usize".into()))?;
                let end = header_end
                    .checked_add(length)
                    .ok_or_else(|| DbfError::Invalid("memo data length overflows usize".into()))?;
                let data = self
                    .bytes
                    .get(header_end..end)
                    .ok_or_else(|| DbfError::Invalid("memo data is truncated".into()))?;
                Ok(Some(data.to_vec()))
            }
            MemoFormat::FoxPro => {
                let header_end = start
                    .checked_add(8)
                    .ok_or_else(|| DbfError::Invalid("memo header overflows usize".into()))?;
                let header = self
                    .bytes
                    .get(start..header_end)
                    .ok_or_else(|| DbfError::Invalid("memo block header is truncated".into()))?;
                let length = usize::try_from(u32::from_be_bytes([
                    header[4], header[5], header[6], header[7],
                ]))
                .map_err(|_| DbfError::Invalid("memo length overflows usize".into()))?;
                let end = header_end
                    .checked_add(length)
                    .ok_or_else(|| DbfError::Invalid("memo data length overflows usize".into()))?;
                let data = self
                    .bytes
                    .get(header_end..end)
                    .ok_or_else(|| DbfError::Invalid("memo data is truncated".into()))?;
                Ok(Some(data.to_vec()))
            }
        }
    }
}

fn find_memo_path(path: &Path) -> Option<std::path::PathBuf> {
    ["fpt", "dbt"]
        .into_iter()
        .map(|extension| path.with_extension(extension))
        .find(|candidate| candidate.is_file())
}

fn memo_index(bytes: &[u8]) -> Result<Option<u32>, DbfError> {
    if bytes.len() == 4 {
        return Ok(Some(u32::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ])));
    }
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\0'))
        .map_or(0, |index| index + 1);
    if end == 0 {
        return Ok(None);
    }
    let text = std::str::from_utf8(&bytes[..end])
        .map_err(|_| DbfError::Invalid("memo block number is not ASCII".into()))?
        .trim_matches([' ', '\0']);
    text.parse::<u32>()
        .map(Some)
        .map_err(|_| DbfError::Invalid("memo block number is not an integer".into()))
}

fn is_memo_field(field_type: u8) -> bool {
    field_type.eq_ignore_ascii_case(&b'M')
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

fn encode_field(
    field: &FieldDescriptor,
    value: &Value,
    language_driver: u8,
) -> Result<Vec<u8>, DbfError> {
    let length = usize::from(field.length);
    match field.field_type.to_ascii_uppercase() {
        b'C' => {
            let bytes = encode_character(value, field, language_driver)?;
            if bytes.len() > length {
                return Err(DbfError::Invalid(format!(
                    "value for {} exceeds field width {}",
                    field.name, field.length
                )));
            }
            let mut output = vec![b' '; length];
            output[..bytes.len()].copy_from_slice(&bytes);
            Ok(output)
        }
        b'D' | b'B' | b'G' | b'M' | b'@' | b'T' => {
            let text = value_text(value, field)?;
            if text.len() > length {
                return Err(DbfError::Invalid(format!(
                    "value for {} exceeds field width {}",
                    field.name, field.length
                )));
            }
            let mut bytes = vec![b' '; length];
            bytes[..text.len()].copy_from_slice(text.as_bytes());
            Ok(bytes)
        }
        b'N' | b'F' => {
            let text = value_text(value, field)?;
            if text.len() > length {
                return Err(DbfError::Invalid(format!(
                    "value for {} exceeds field width {}",
                    field.name, field.length
                )));
            }
            let mut bytes = vec![b' '; length];
            let start = length - text.len();
            bytes[start..].copy_from_slice(text.as_bytes());
            Ok(bytes)
        }
        b'L' => {
            let marker = match value {
                Value::Null => b' ',
                Value::Bool(true) => b'T',
                Value::Bool(false) => b'F',
                Value::String(text) if text.len() == 1 => text.as_bytes()[0].to_ascii_uppercase(),
                _ => {
                    return Err(DbfError::Invalid(format!(
                        "logical field {} requires boolean, one-byte text, or null",
                        field.name
                    )));
                }
            };
            if !matches!(marker, b' ' | b'T' | b'F' | b'Y' | b'N') {
                return Err(DbfError::Invalid(format!(
                    "invalid logical marker for {}",
                    field.name
                )));
            }
            Ok(vec![marker; length])
        }
        b'I' | b'+' => {
            if length != 4 {
                return Err(DbfError::Invalid(format!(
                    "integer field {} must be four bytes",
                    field.name
                )));
            }
            let integer = value_i64(value, field)?;
            let integer = i32::try_from(integer).map_err(|_| {
                DbfError::Invalid(format!("integer value for {} is out of range", field.name))
            })?;
            Ok(integer.to_le_bytes().to_vec())
        }
        b'O' => {
            if length != 8 {
                return Err(DbfError::Invalid(format!(
                    "double field {} must be eight bytes",
                    field.name
                )));
            }
            let number = value_f64(value, field)?;
            Ok(number.to_le_bytes().to_vec())
        }
        field_type => Err(DbfError::Invalid(format!(
            "writing field type 0x{field_type:02x} is unsupported"
        ))),
    }
}

fn encode_character(
    value: &Value,
    field: &FieldDescriptor,
    language_driver: u8,
) -> Result<Vec<u8>, DbfError> {
    let text = value_text(value, field)?;
    match language_driver {
        0x03 | 0x57 => encode_windows_1252(&text).ok_or_else(|| {
            DbfError::Invalid(format!(
                "value for {} contains a character outside Windows-1252",
                field.name
            ))
        }),
        _ => Ok(text.into_bytes()),
    }
}

fn value_text(value: &Value, field: &FieldDescriptor) -> Result<String, DbfError> {
    match value {
        Value::Null => Ok(String::new()),
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(boolean) => Ok(boolean.to_string()),
        _ => Err(DbfError::Invalid(format!(
            "field {} requires a scalar value",
            field.name
        ))),
    }
}

fn value_i64(value: &Value, field: &FieldDescriptor) -> Result<i64, DbfError> {
    match value {
        Value::Null => Ok(0),
        Value::Number(number) => number
            .as_i64()
            .ok_or_else(|| DbfError::Invalid(format!("field {} requires an integer", field.name))),
        Value::String(text) => text
            .trim()
            .parse::<i64>()
            .map_err(|_| DbfError::Invalid(format!("field {} requires an integer", field.name))),
        _ => Err(DbfError::Invalid(format!(
            "field {} requires an integer",
            field.name
        ))),
    }
}

fn value_f64(value: &Value, field: &FieldDescriptor) -> Result<f64, DbfError> {
    let number = match value {
        Value::Null => 0.0,
        Value::Number(number) => number.as_f64().ok_or_else(|| {
            DbfError::Invalid(format!("field {} requires a finite number", field.name))
        })?,
        Value::String(text) => text.trim().parse::<f64>().map_err(|_| {
            DbfError::Invalid(format!("field {} requires a finite number", field.name))
        })?,
        _ => {
            return Err(DbfError::Invalid(format!(
                "field {} requires a number",
                field.name
            )));
        }
    };
    if number.is_finite() {
        Ok(number)
    } else {
        Err(DbfError::Invalid(format!(
            "field {} requires a finite number",
            field.name
        )))
    }
}

fn parse_fields(
    bytes: &[u8],
    start: usize,
    end: usize,
    descriptor_size: usize,
) -> Result<Vec<FieldDescriptor>, DbfError> {
    if (end - start) % descriptor_size != 0 {
        return Err(DbfError::Invalid(
            "field descriptor area is misaligned".into(),
        ));
    }

    let (name_size, type_offset, length_offset, decimal_offset) =
        if descriptor_size == CLASSIC_DESCRIPTOR_SIZE {
            (11, 11, 16, 17)
        } else {
            (32, 32, 33, 34)
        };
    let mut names = BTreeSet::new();
    let mut offset = 1;
    let mut fields = Vec::with_capacity((end - start) / descriptor_size);
    for descriptor in bytes[start..end].chunks_exact(descriptor_size) {
        let name_end = descriptor[..name_size]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(name_size);
        let name = String::from_utf8_lossy(&descriptor[..name_end])
            .trim()
            .to_owned();
        if name.is_empty() || !names.insert(name.clone()) {
            return Err(DbfError::Invalid(
                "field names must be non-empty and unique".into(),
            ));
        }
        let length = descriptor[length_offset];
        if length == 0 {
            return Err(DbfError::Invalid(format!("field {name} has zero width")));
        }
        fields.push(FieldDescriptor {
            name,
            field_type: descriptor[type_offset],
            length,
            decimal_count: descriptor[decimal_offset],
            offset,
        });
        offset = offset
            .checked_add(length as usize)
            .ok_or_else(|| DbfError::Invalid("field offsets overflow usize".into()))?;
    }
    Ok(fields)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, DbfError> {
    let bytes = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| DbfError::Invalid("header is truncated".into()))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, DbfError> {
    let bytes = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| DbfError::Invalid("header is truncated".into()))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn decode_field(field_type: u8, bytes: &[u8], language_driver: u8) -> Value {
    match field_type.to_ascii_uppercase() {
        b'C' => Value::String(text(bytes, language_driver)),
        b'D' | b'B' | b'G' | b'M' => Value::String(text(bytes, 0)),
        b'F' | b'N' => numeric(bytes),
        b'L' => match bytes.first().map(|byte| byte.to_ascii_uppercase()) {
            Some(b'T') | Some(b'Y') => Value::Bool(true),
            Some(b'F') | Some(b'N') => Value::Bool(false),
            _ => Value::Null,
        },
        b'I' | b'+' if bytes.len() >= 4 => {
            Value::Number(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).into())
        }
        b'O' if bytes.len() >= 8 => {
            let value = f64::from_le_bytes(bytes[..8].try_into().unwrap());
            if value.is_finite() {
                Number::from_f64(value)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        b'@' | b'T' => Value::String(hex(bytes)),
        _ => Value::String(text(bytes, language_driver)),
    }
}

fn text(bytes: &[u8], language_driver: u8) -> String {
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\0'))
        .map_or(0, |index| index + 1);
    let bytes = &bytes[..end];
    if matches!(language_driver, 0x03 | 0x57) {
        return decode_windows_1252(bytes);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn decode_windows_1252(bytes: &[u8]) -> String {
    const EXTENDED: [char; 32] = [
        '€', '\u{fffd}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{fffd}', 'Ž',
        '\u{fffd}', '\u{fffd}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ',
        '\u{fffd}', 'ž', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|byte| match byte {
            0x00..=0x7f => char::from(*byte),
            0x80..=0x9f => EXTENDED[usize::from(*byte - 0x80)],
            byte => char::from_u32(u32::from(*byte)).expect("Windows-1252 byte is valid"),
        })
        .collect()
}

fn encode_windows_1252(text: &str) -> Option<Vec<u8>> {
    text.chars()
        .map(|character| match character {
            '\u{0000}'..='\u{007f}' | '\u{00a0}'..='\u{00ff}' => Some(character as u32 as u8),
            '€' => Some(0x80),
            '‚' => Some(0x82),
            'ƒ' => Some(0x83),
            '„' => Some(0x84),
            '…' => Some(0x85),
            '†' => Some(0x86),
            '‡' => Some(0x87),
            'ˆ' => Some(0x88),
            '‰' => Some(0x89),
            'Š' => Some(0x8a),
            '‹' => Some(0x8b),
            'Œ' => Some(0x8c),
            'Ž' => Some(0x8e),
            '‘' => Some(0x91),
            '’' => Some(0x92),
            '“' => Some(0x93),
            '”' => Some(0x94),
            '•' => Some(0x95),
            '–' => Some(0x96),
            '—' => Some(0x97),
            '˜' => Some(0x98),
            '™' => Some(0x99),
            'š' => Some(0x9a),
            '›' => Some(0x9b),
            'œ' => Some(0x9c),
            'ž' => Some(0x9e),
            'Ÿ' => Some(0x9f),
            _ => None,
        })
        .collect()
}

fn numeric(bytes: &[u8]) -> Value {
    let value = text(bytes, 0).trim().to_owned();
    if value.is_empty() {
        return Value::Null;
    }
    if let Ok(integer) = value.parse::<i64>() {
        return Value::Number(integer.into());
    }
    if let Ok(float) = value.parse::<f64>() {
        if let Some(number) = Number::from_f64(float) {
            return Value::Number(number);
        }
    }
    Value::String(value)
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        include_str!("../tests/fixtures/users.dbf.hex")
            .split_whitespace()
            .map(|token| u8::from_str_radix(token, 16).unwrap())
            .collect()
    }

    #[test]
    fn reads_header_fields_and_active_records() {
        let table = DbfTable::from_bytes(&fixture()).unwrap();

        assert_eq!(table.header.record_count, 2);
        assert_eq!(table.header.record_length, 18);
        assert_eq!(table.fields[0].name, "ID");
        assert_eq!(table.fields[1].offset, 4);
        assert!(table.records()[1].deleted);
        assert_eq!(table.active_json().len(), 1);
        assert_eq!(table.active_json()[0]["NAME"], "Alice");
        assert_eq!(table.active_json()[0]["AGE"], 29);
    }

    #[test]
    fn rejects_truncated_records() {
        let mut bytes = fixture();
        bytes.pop();
        bytes.pop();
        assert!(matches!(
            DbfTable::from_bytes(&bytes),
            Err(DbfError::Invalid(message)) if message == "record area is truncated"
        ));
    }

    #[test]
    fn rejects_unknown_deletion_markers() {
        let mut bytes = fixture();
        bytes[161] = b'!';
        assert!(matches!(
            DbfTable::from_bytes(&bytes),
            Err(DbfError::Invalid(message)) if message.contains("unknown deletion marker")
        ));
    }

    #[test]
    fn reads_level7_descriptor_layout() {
        let mut bytes = vec![0; 122];
        bytes[0] = 0x04;
        bytes[4] = 1;
        bytes[8] = 117;
        bytes[10] = 4;
        bytes[68] = b'V';
        bytes[100] = b'C';
        bytes[101] = 3;
        bytes[116] = FIELD_TERMINATOR;
        bytes[117..121].copy_from_slice(b" yes");
        bytes[121] = 0x1a;

        let table = DbfTable::from_bytes(&bytes).unwrap();
        assert_eq!(table.fields[0].name, "V");
        assert_eq!(table.active_json()[0]["V"], "yes");
    }

    #[test]
    fn decodes_float_fields_as_numbers() {
        assert_eq!(decode_field(b'F', b" 1.5", 0), serde_json::json!(1.5));
    }

    #[test]
    fn decodes_and_encodes_windows_1252_character_fields() {
        let mut bytes = fixture();
        bytes[29] = 0x03;
        let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
        let name_start = record_start + 4;
        bytes[name_start..name_start + 10].fill(b' ');
        bytes[name_start] = 0xe9;

        let mut table = DbfTable::from_bytes(&bytes).unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], "é");

        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "€"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        assert_eq!(table.to_bytes()[name_start], 0x80);
        assert_eq!(
            DbfTable::from_bytes(&table.to_bytes())
                .unwrap()
                .active_record(1)
                .unwrap()
                .values["NAME"],
            "€"
        );

        let error = table
            .patch_record(
                1,
                serde_json::json!({"NAME": "漢"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("outside Windows-1252"));
    }

    #[test]
    fn mutates_records_and_round_trips_to_dbf() {
        let mut table = DbfTable::from_bytes(&fixture()).unwrap();
        let inserted = table
            .insert_record(
                serde_json::json!({
                    "ID": 3,
                    "NAME": "Carol",
                    "AGE": 42,
                    "ACTIVE": false
                })
                .as_object()
                .unwrap()
                .clone(),
            )
            .unwrap();
        assert_eq!(inserted, 3);

        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "Alicia"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        table
            .replace_record(
                3,
                serde_json::json!({
                    "ID": 3,
                    "NAME": "Carol",
                    "AGE": 43,
                    "ACTIVE": true
                })
                .as_object()
                .unwrap()
                .clone(),
            )
            .unwrap();
        table.delete_record(1).unwrap();

        let round_trip = DbfTable::from_bytes(&table.to_bytes()).unwrap();
        assert_eq!(round_trip.header.record_count, 3);
        assert!(round_trip.records()[0].deleted);
        assert_eq!(round_trip.active_record(3).unwrap().values["NAME"], "Carol");
        assert_eq!(round_trip.active_record(3).unwrap().values["AGE"], 43);
    }

    #[test]
    fn rejects_unknown_mutation_fields_without_changing_table() {
        let mut table = DbfTable::from_bytes(&fixture()).unwrap();
        let before = table.to_bytes();
        let error = table
            .patch_record(
                1,
                serde_json::json!({"UNKNOWN": true})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap_err();

        assert!(error.to_string().contains("unknown field UNKNOWN"));
        assert_eq!(table.to_bytes(), before);
    }

    #[test]
    fn reads_dbase3_memo_sidecar_and_preserves_pointer_on_mutation() {
        let path =
            std::env::temp_dir().join(format!("txbase-dbase3-memo-{}.dbf", std::process::id()));
        let memo_path = path.with_extension("dbt");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&memo_path);

        let mut bytes = fixture();
        bytes[0] = 0x83;
        bytes[64 + 11] = b'M';
        let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
        let memo_start = record_start + 4;
        bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
        fs::write(&path, bytes).unwrap();

        let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
        let text = b"memo from dbt";
        memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + text.len()].copy_from_slice(text);
        memo[DBT_BLOCK_SIZE + text.len()] = EOF_MARKER;
        fs::write(&memo_path, memo).unwrap();

        let mut table = DbfTable::from_path(&path).unwrap();
        assert_eq!(
            table.active_record(1).unwrap().values["NAME"],
            "memo from dbt"
        );
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "memo from dbt"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        let error = table
            .patch_record(
                1,
                serde_json::json!({"NAME": "new memo"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("memo field NAME cannot be changed")
        );
        table
            .patch_record(
                1,
                serde_json::json!({"AGE": 30}).as_object().unwrap().clone(),
            )
            .unwrap();
        table.save_with_wal(&path).unwrap();

        let reread = DbfTable::from_path(&path).unwrap();
        assert_eq!(
            reread.active_record(1).unwrap().values["NAME"],
            "memo from dbt"
        );
        assert_eq!(reread.active_record(1).unwrap().values["AGE"], 30);
        assert_eq!(
            &reread.to_bytes()[memo_start..memo_start + 10],
            b"         1"
        );

        fs::remove_file(path).unwrap();
        fs::remove_file(memo_path).unwrap();
    }

    #[test]
    fn reads_foxpro_fpt_memo_sidecar() {
        let path =
            std::env::temp_dir().join(format!("txbase-foxpro-memo-{}.dbf", std::process::id()));
        let memo_path = path.with_extension("fpt");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&memo_path);

        let mut bytes = fixture();
        bytes[0] = 0xf5;
        bytes[64 + 11] = b'M';
        let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
        let memo_start = record_start + 4;
        bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
        fs::write(&path, bytes).unwrap();

        let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
        memo[6..8].copy_from_slice(&(DBT_BLOCK_SIZE as u16).to_be_bytes());
        memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&1u32.to_be_bytes());
        let text = b"memo from fpt";
        memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
            .copy_from_slice(&(text.len() as u32).to_be_bytes());
        memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + text.len()].copy_from_slice(text);
        fs::write(&memo_path, memo).unwrap();

        let table = DbfTable::from_path(&path).unwrap();
        assert_eq!(
            table.active_record(1).unwrap().values["NAME"],
            "memo from fpt"
        );

        fs::remove_file(path).unwrap();
        fs::remove_file(memo_path).unwrap();
    }

    #[test]
    fn recovers_latest_snapshot_from_wal_before_reading() {
        let path = std::env::temp_dir().join(format!(
            "txbase-dbf-recovery-{}-{}.dbf",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let wal_path = path.with_extension("txbase.wal");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&wal_path);

        let mut pending = DbfTable::from_bytes(&fixture()).unwrap();
        pending
            .insert_record(
                serde_json::json!({
                    "ID": 3,
                    "NAME": "Carol",
                    "AGE": 42,
                    "ACTIVE": true
                })
                .as_object()
                .unwrap()
                .clone(),
            )
            .unwrap();
        fs::write(&path, fixture()).unwrap();

        let mut wal = FileWal::open(&wal_path).unwrap();
        let mut payload = SNAPSHOT_MAGIC.to_vec();
        payload.extend_from_slice(&pending.to_bytes());
        wal.append(&payload).unwrap();
        wal.sync().unwrap();
        drop(wal);

        let recovered = DbfTable::from_path(&path).unwrap();
        assert_eq!(recovered.active_record(3).unwrap().values["NAME"], "Carol");
        assert!(!wal_path.exists());

        fs::remove_file(path).unwrap();
    }
}
