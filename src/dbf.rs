use crate::transaction::{FileWal, TransactionError, Wal};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, BTreeSet};
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
const MEMO_SNAPSHOT_MAGIC: &[u8; 4] = b"TXDM";
const ACTIVE_RECORD: u8 = 0x20;
const DELETED_RECORD: u8 = 0x2a;
const DBT_BLOCK_SIZE: usize = 512;
const CP437_UPPER: &str = concat!(
    "\u{c7}\u{fc}\u{e9}\u{e2}\u{e4}\u{e0}\u{e5}\u{e7}\u{ea}\u{eb}\u{e8}\u{ef}\u{ee}\u{ec}\u{c4}\u{c5}\u{c9}\u{e6}\u{c6}\u{f4}\u{f6}\u{f2}\u{fb}\u{f9}\u{ff}\u{d6}\u{dc}\u{a2}\u{a3}\u{a5}\u{20a7}\u{192}",
    "\u{e1}\u{ed}\u{f3}\u{fa}\u{f1}\u{d1}\u{aa}\u{ba}\u{bf}\u{2310}\u{ac}\u{bd}\u{bc}\u{a1}\u{ab}\u{bb}\u{2591}\u{2592}\u{2593}\u{2502}\u{2524}\u{2561}\u{2562}\u{2556}\u{2555}\u{2563}\u{2551}\u{2557}\u{255d}\u{255c}\u{255b}\u{2510}",
    "\u{2514}\u{2534}\u{252c}\u{251c}\u{2500}\u{253c}\u{255e}\u{255f}\u{255a}\u{2554}\u{2569}\u{2566}\u{2560}\u{2550}\u{256c}\u{2567}\u{2568}\u{2564}\u{2565}\u{2559}\u{2558}\u{2552}\u{2553}\u{256b}\u{256a}\u{2518}\u{250c}\u{2588}\u{2584}\u{258c}\u{2590}\u{2580}",
    "\u{3b1}\u{df}\u{393}\u{3c0}\u{3a3}\u{3c3}\u{b5}\u{3c4}\u{3a6}\u{398}\u{3a9}\u{3b4}\u{221e}\u{3c6}\u{3b5}\u{2229}\u{2261}\u{b1}\u{2265}\u{2264}\u{2320}\u{2321}\u{f7}\u{2248}\u{b0}\u{2219}\u{b7}\u{221a}\u{207f}\u{b2}\u{25a0}\u{a0}"
);
const CP850_UPPER: &str = concat!(
    "\u{c7}\u{fc}\u{e9}\u{e2}\u{e4}\u{e0}\u{e5}\u{e7}\u{ea}\u{eb}\u{e8}\u{ef}\u{ee}\u{ec}\u{c4}\u{c5}\u{c9}\u{e6}\u{c6}\u{f4}\u{f6}\u{f2}\u{fb}\u{f9}\u{ff}\u{d6}\u{dc}\u{f8}\u{a3}\u{d8}\u{d7}\u{192}",
    "\u{e1}\u{ed}\u{f3}\u{fa}\u{f1}\u{d1}\u{aa}\u{ba}\u{bf}\u{ae}\u{ac}\u{bd}\u{bc}\u{a1}\u{ab}\u{bb}\u{2591}\u{2592}\u{2593}\u{2502}\u{2524}\u{c1}\u{c2}\u{c0}\u{a9}\u{2563}\u{2551}\u{2557}\u{255d}\u{a2}\u{a5}",
    "\u{2510}\u{2514}\u{2534}\u{252c}\u{251c}\u{2500}\u{253c}\u{e3}\u{c3}\u{255a}\u{2554}\u{2569}\u{2566}\u{2560}\u{2550}\u{256c}\u{a4}\u{f0}\u{d0}\u{ca}\u{cb}\u{c8}\u{131}\u{cd}\u{ce}\u{cf}\u{2518}\u{250c}\u{2588}\u{2584}\u{a6}\u{cc}\u{2580}",
    "\u{d3}\u{df}\u{d4}\u{d2}\u{f5}\u{d5}\u{b5}\u{fe}\u{de}\u{da}\u{db}\u{d9}\u{fd}\u{dd}\u{af}\u{b4}\u{ad}\u{b1}\u{2017}\u{be}\u{b6}\u{a7}\u{f7}\u{b8}\u{b0}\u{a8}\u{b7}\u{b9}\u{b3}\u{b2}\u{25a0}\u{a0}"
);
const CP852_UPPER: &str = concat!(
    "\u{c7}\u{fc}\u{e9}\u{e2}\u{e4}\u{16f}\u{107}\u{e7}\u{142}\u{eb}\u{150}\u{151}\u{ee}\u{179}\u{c4}\u{106}\u{c9}\u{139}\u{13a}\u{f4}\u{f6}\u{13d}\u{13e}\u{15a}\u{15b}\u{d6}\u{dc}\u{164}\u{165}\u{141}\u{d7}\u{10d}",
    "\u{e1}\u{ed}\u{f3}\u{fa}\u{104}\u{105}\u{17d}\u{17e}\u{118}\u{119}\u{ac}\u{17a}\u{10c}\u{15f}\u{ab}\u{bb}\u{2591}\u{2592}\u{2593}\u{2502}\u{2524}\u{c1}\u{c2}\u{11a}\u{15e}\u{2563}\u{2551}\u{2557}\u{255d}\u{17b}\u{17c}\u{2510}",
    "\u{2514}\u{2534}\u{252c}\u{251c}\u{2500}\u{253c}\u{102}\u{103}\u{255a}\u{2554}\u{2569}\u{2566}\u{2560}\u{2550}\u{256c}\u{a4}\u{111}\u{110}\u{10e}\u{cb}\u{10f}\u{147}\u{cd}\u{ce}\u{11b}\u{2518}\u{250c}\u{2588}\u{2584}\u{162}\u{16e}\u{2580}",
    "\u{d3}\u{df}\u{d4}\u{143}\u{144}\u{148}\u{160}\u{161}\u{154}\u{da}\u{155}\u{170}\u{fd}\u{dd}\u{163}\u{b4}\u{ad}\u{2dd}\u{2db}\u{2c7}\u{2d8}\u{a7}\u{f7}\u{b8}\u{b0}\u{a8}\u{2d9}\u{171}\u{158}\u{159}\u{25a0}\u{a0}"
);
const CP866_UPPER: &str = concat!(
    "\u{410}\u{411}\u{412}\u{413}\u{414}\u{415}\u{416}\u{417}\u{418}\u{419}\u{41a}\u{41b}\u{41c}\u{41d}\u{41e}\u{41f}\u{420}\u{421}\u{422}\u{423}\u{424}\u{425}\u{426}\u{427}\u{428}\u{429}\u{42a}\u{42b}\u{42c}\u{42d}\u{42e}\u{42f}",
    "\u{430}\u{431}\u{432}\u{433}\u{434}\u{435}\u{436}\u{437}\u{438}\u{439}\u{43a}\u{43b}\u{43c}\u{43d}\u{43e}\u{43f}\u{2591}\u{2592}\u{2593}\u{2502}\u{2524}\u{2561}\u{2562}\u{2556}\u{2555}\u{2563}\u{2551}\u{2557}\u{255d}\u{255c}\u{255b}\u{2510}",
    "\u{2514}\u{2534}\u{252c}\u{251c}\u{2500}\u{253c}\u{255e}\u{255f}\u{255a}\u{2554}\u{2569}\u{2566}\u{2560}\u{2550}\u{256c}\u{2567}\u{2568}\u{2564}\u{2565}\u{2559}\u{2558}\u{2552}\u{2553}\u{256b}\u{256a}\u{2518}\u{250c}\u{2588}\u{2584}\u{258c}\u{2590}\u{2580}",
    "\u{440}\u{441}\u{442}\u{443}\u{444}\u{445}\u{446}\u{447}\u{448}\u{449}\u{44a}\u{44b}\u{44c}\u{44d}\u{44e}\u{44f}\u{401}\u{451}\u{404}\u{454}\u{407}\u{457}\u{40e}\u{45e}\u{b0}\u{2219}\u{b7}\u{221a}\u{2116}\u{a4}\u{25a0}\u{a0}"
);

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
struct RecoverySnapshot {
    dbf: Vec<u8>,
    memo: Option<MemoSnapshot>,
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
}

impl DbfTable {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, DbfError> {
        let path = path.as_ref();
        Self::recover_wal(path)?;
        let mut table = Self::from_bytes(&fs::read(path)?)?;
        if table.has_sidecar_fields() {
            if let Some(memo_path) = find_memo_path(path) {
                let memo = MemoFile::open(&memo_path, table.header.version)?;
                table.resolve_memos(&memo)?;
                table.memo = Some(memo);
            }
        }
        Ok(table)
    }

    fn has_sidecar_fields(&self) -> bool {
        self.fields
            .iter()
            .any(|field| is_sidecar_field(field.field_type))
    }

    fn resolve_memos(&mut self, memo: &MemoFile) -> Result<(), DbfError> {
        let fields = self
            .fields
            .iter()
            .filter(|field| is_sidecar_field(field.field_type))
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
                let value = if is_memo_field(field.field_type) {
                    Value::String(text(&data, self.header.language_driver))
                } else {
                    Value::String(hex(&data))
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
            memo_updates: BTreeMap::new(),
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
        save_bytes_to(path, &self.bytes, "txbase.tmp")
    }

    pub fn save_with_wal(&mut self, path: impl AsRef<Path>) -> Result<(), DbfError> {
        let path = path.as_ref();
        let mut prepared = self.clone();
        let memo_snapshot = prepared.apply_memo_updates(path)?;
        let wal_path = path.with_extension("txbase.wal");
        let mut wal = FileWal::open(&wal_path).map_err(transaction_error)?;
        let payload = match &memo_snapshot {
            Some(memo) => memo_snapshot_payload(&prepared.bytes, memo)?,
            None => snapshot_payload(&prepared.bytes),
        };
        wal.append(&payload).map_err(transaction_error)?;
        wal.sync().map_err(transaction_error)?;
        if let Some(memo) = &memo_snapshot {
            save_bytes_to(
                &path.with_extension(memo.format.extension()),
                &memo.bytes,
                "txbase.memo.tmp",
            )?;
        }
        save_bytes_to(path, &prepared.bytes, "txbase.tmp")?;
        if wal.clear().is_ok() {
            drop(wal);
            let _ = fs::remove_file(wal_path);
        }
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
            wal.records()
                .iter()
                .rev()
                .find_map(|(_, payload)| match decode_snapshot(payload) {
                    Ok(Some(snapshot)) => Some(Ok(snapshot)),
                    Ok(None) => None,
                    Err(error) => Some(Err(error)),
                });
        let Some(snapshot) = snapshot else {
            return Ok(());
        };
        let snapshot = snapshot?;
        Self::from_bytes(&snapshot.dbf)?;
        if let Some(memo) = &snapshot.memo {
            save_bytes_to(
                &path.with_extension(memo.format.extension()),
                &memo.bytes,
                "txbase.memo.tmp",
            )?;
        }
        save_bytes_to(path, &snapshot.dbf, "txbase.tmp")?;
        if wal.clear().is_ok() {
            drop(wal);
            let _ = fs::remove_file(wal_path);
        }
        Ok(())
    }

    pub fn insert_record(&mut self, values: Map<String, Value>) -> Result<usize, DbfError> {
        let values = self.normalize_values(&values)?;
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
            for field in self
                .fields
                .iter()
                .filter(|field| is_sidecar_field(field.field_type))
            {
                let value = values.get(&field.name).unwrap_or(&Value::Null);
                storage_values.insert(field.name.clone(), empty_memo_value(field));
                if let Some(update) = sidecar_update(value, field, memo_format)? {
                    memo_updates.insert((number - 1, field.name.clone()), update);
                }
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
        let values = self.normalize_values(&values)?;
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
        let values = self.normalize_values(&values)?;
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

    fn prepare_existing_storage(
        &self,
        index: usize,
        values: &Map<String, Value>,
        changed_fields: Option<&BTreeSet<String>>,
    ) -> Result<PreparedStorage, DbfError> {
        let mut storage_values = values.clone();
        let mut memo_updates = self.memo_updates.clone();
        for field in self
            .fields
            .iter()
            .filter(|field| is_sidecar_field(field.field_type))
        {
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

            if self.memo.is_some() {
                if is_memo_field(field.field_type) {
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
                .find(|field| field.name == field_name && is_sidecar_field(field.field_type))
                .cloned()
                .ok_or_else(|| {
                    DbfError::Invalid(format!("sidecar field {field_name} not found"))
                })?;
            let pointer = match value {
                MemoUpdate::Text(value) => {
                    if value.is_empty() {
                        encode_memo_pointer(&field, 0)?
                    } else {
                        let bytes = encode_character(
                            &Value::String(value),
                            &field,
                            self.header.language_driver,
                        )?;
                        let block = memo.append_text(&bytes)?;
                        encode_memo_pointer(&field, block)?
                    }
                }
                MemoUpdate::Binary(bytes) => {
                    if bytes.is_empty() {
                        encode_memo_pointer(&field, 0)?
                    } else {
                        let block = memo.append_binary(&bytes)?;
                        encode_memo_pointer(&field, block)?
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
                decode_field(field.field_type, target, self.header.language_driver),
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

fn snapshot_payload(dbf: &[u8]) -> Vec<u8> {
    let mut payload = SNAPSHOT_MAGIC.to_vec();
    payload.extend_from_slice(dbf);
    payload
}

fn memo_snapshot_payload(dbf: &[u8], memo: &MemoSnapshot) -> Result<Vec<u8>, DbfError> {
    let dbf_length = u64::try_from(dbf.len())
        .map_err(|_| DbfError::Invalid("DBF snapshot length overflows u64".into()))?;
    let memo_length = u64::try_from(memo.bytes.len())
        .map_err(|_| DbfError::Invalid("memo snapshot length overflows u64".into()))?;
    let mut payload = MEMO_SNAPSHOT_MAGIC.to_vec();
    payload.push(memo.format.tag());
    payload.extend_from_slice(&dbf_length.to_le_bytes());
    payload.extend_from_slice(&memo_length.to_le_bytes());
    payload.extend_from_slice(dbf);
    payload.extend_from_slice(&memo.bytes);
    Ok(payload)
}

fn decode_snapshot(payload: &[u8]) -> Result<Option<RecoverySnapshot>, DbfError> {
    if let Some(dbf) = payload.strip_prefix(SNAPSHOT_MAGIC) {
        return Ok(Some(RecoverySnapshot {
            dbf: dbf.to_vec(),
            memo: None,
        }));
    }
    let Some(body) = payload.strip_prefix(MEMO_SNAPSHOT_MAGIC) else {
        return Ok(None);
    };
    let header = body
        .get(..17)
        .ok_or_else(|| DbfError::Invalid("memo snapshot header is truncated".into()))?;
    let format = MemoFormat::from_tag(header[0])?;
    let dbf_length = usize::try_from(u64::from_le_bytes(
        header[1..9]
            .try_into()
            .expect("memo snapshot DBF length is fixed"),
    ))
    .map_err(|_| DbfError::Invalid("DBF snapshot length overflows usize".into()))?;
    let memo_length = usize::try_from(u64::from_le_bytes(
        header[9..17]
            .try_into()
            .expect("memo snapshot memo length is fixed"),
    ))
    .map_err(|_| DbfError::Invalid("memo snapshot length overflows usize".into()))?;
    let memo_start = 17usize
        .checked_add(dbf_length)
        .ok_or_else(|| DbfError::Invalid("memo snapshot length overflows usize".into()))?;
    let end = memo_start
        .checked_add(memo_length)
        .ok_or_else(|| DbfError::Invalid("memo snapshot length overflows usize".into()))?;
    if end != body.len() {
        return Err(DbfError::Invalid(
            "memo snapshot length does not match payload".into(),
        ));
    }
    Ok(Some(RecoverySnapshot {
        dbf: body[17..memo_start].to_vec(),
        memo: Some(MemoSnapshot {
            format,
            bytes: body[memo_start..end].to_vec(),
        }),
    }))
}

fn save_bytes_to(path: &Path, bytes: &[u8], temporary_extension: &str) -> Result<(), DbfError> {
    let temporary_path = path.with_extension(temporary_extension);
    let mut file = fs::File::create(&temporary_path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temporary_path, path)?;
    Ok(())
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
                let total_length = usize::try_from(u32::from_le_bytes([
                    header[4], header[5], header[6], header[7],
                ]))
                .map_err(|_| DbfError::Invalid("memo length overflows usize".into()))?;
                let length = total_length.checked_sub(8).ok_or_else(|| {
                    DbfError::Invalid("dBASE IV memo length is smaller than its header".into())
                })?;
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

    fn append_text(&mut self, data: &[u8]) -> Result<u32, DbfError> {
        let mut payload = Vec::new();
        match self.format {
            MemoFormat::Dbase3 => {
                if data.contains(&EOF_MARKER) {
                    return Err(DbfError::Invalid(
                        "dBASE III memo text contains the end marker".into(),
                    ));
                }
                payload.extend_from_slice(data);
                payload.push(EOF_MARKER);
            }
            MemoFormat::Dbase4 => {
                let length = u32::try_from(
                    data.len()
                        .checked_add(8)
                        .ok_or_else(|| DbfError::Invalid("memo text is too long".into()))?,
                )
                .map_err(|_| DbfError::Invalid("memo text is too long".into()))?;
                payload.extend_from_slice(&[0xff, 0xff, 0x08, 0x00]);
                payload.extend_from_slice(&length.to_le_bytes());
                payload.extend_from_slice(data);
            }
            MemoFormat::FoxPro => {
                let length = u32::try_from(data.len())
                    .map_err(|_| DbfError::Invalid("memo text is too long".into()))?;
                payload.extend_from_slice(&1u32.to_be_bytes());
                payload.extend_from_slice(&length.to_be_bytes());
                payload.extend_from_slice(data);
            }
        }
        self.append_payload(payload)
    }

    fn append_binary(&mut self, data: &[u8]) -> Result<u32, DbfError> {
        let length = u32::try_from(data.len())
            .map_err(|_| DbfError::Invalid("binary data is too long".into()))?;
        let mut payload = Vec::with_capacity(8usize.saturating_add(data.len()));
        match self.format {
            MemoFormat::Dbase4 => {
                let total_length = length
                    .checked_add(8)
                    .ok_or_else(|| DbfError::Invalid("binary data is too long".into()))?;
                payload.extend_from_slice(&[0xff, 0xff, 0x08, 0x00]);
                payload.extend_from_slice(&total_length.to_le_bytes());
            }
            MemoFormat::FoxPro => {
                payload.extend_from_slice(&0u32.to_be_bytes());
                payload.extend_from_slice(&length.to_be_bytes());
            }
            MemoFormat::Dbase3 => {
                return Err(DbfError::Invalid(
                    "binary sidecar writes require dBASE IV DBT or FPT".into(),
                ));
            }
        }
        payload.extend_from_slice(data);
        self.append_payload(payload)
    }

    fn append_payload(&mut self, mut payload: Vec<u8>) -> Result<u32, DbfError> {
        if self.bytes.len() < 4 {
            return Err(DbfError::Invalid("memo header is truncated".into()));
        }
        let start_block = self.bytes.len() / self.block_size
            + usize::from(self.bytes.len() % self.block_size != 0);
        let aligned_length = start_block
            .checked_mul(self.block_size)
            .ok_or_else(|| DbfError::Invalid("memo block offset overflows usize".into()))?;
        if self.bytes.len() < aligned_length {
            self.bytes.resize(aligned_length, 0);
        }
        let block_count =
            payload.len() / self.block_size + usize::from(payload.len() % self.block_size != 0);
        let padded_length = block_count
            .checked_mul(self.block_size)
            .ok_or_else(|| DbfError::Invalid("memo text is too long".into()))?;
        let padding = if self.format == MemoFormat::FoxPro {
            0
        } else {
            b' '
        };
        payload.resize(padded_length, padding);
        self.bytes.extend_from_slice(&payload);

        let next_block = start_block
            .checked_add(block_count)
            .and_then(|block| u32::try_from(block).ok())
            .ok_or_else(|| DbfError::Invalid("memo block number overflows u32".into()))?;
        self.bytes[0..4].copy_from_slice(&next_block.to_be_bytes());
        u32::try_from(start_block)
            .map_err(|_| DbfError::Invalid("memo block number overflows u32".into()))
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

fn is_binary_field(field_type: u8) -> bool {
    matches!(field_type.to_ascii_uppercase(), b'B' | b'G')
}

fn is_sidecar_field(field_type: u8) -> bool {
    is_memo_field(field_type) || is_binary_field(field_type)
}

fn sidecar_update(
    value: &Value,
    field: &FieldDescriptor,
    memo_format: MemoFormat,
) -> Result<Option<MemoUpdate>, DbfError> {
    if is_memo_field(field.field_type) {
        let text = value_text(value, field)?;
        return Ok((!text.is_empty()).then_some(MemoUpdate::Text(text)));
    }

    let bytes = binary_value(value, field)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if memo_format == MemoFormat::Dbase3 {
        return Err(DbfError::Invalid(
            "binary sidecar writes require dBASE IV DBT or FPT".into(),
        ));
    }
    Ok(Some(MemoUpdate::Binary(bytes)))
}

fn binary_value(value: &Value, field: &FieldDescriptor) -> Result<Vec<u8>, DbfError> {
    let Some(text) = value.as_str() else {
        if value.is_null() {
            return Ok(Vec::new());
        }
        return Err(DbfError::Invalid(format!(
            "binary field {} requires an even-length hexadecimal string or null",
            field.name
        )));
    };
    let bytes = text.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(DbfError::Invalid(format!(
            "binary field {} requires an even-length hexadecimal string",
            field.name
        )));
    }
    bytes
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_digit(pair[0]);
            let low = hex_digit(pair[1]);
            match (high, low) {
                (Some(high), Some(low)) => Ok(high << 4 | low),
                _ => Err(DbfError::Invalid(format!(
                    "binary field {} contains a non-hexadecimal character",
                    field.name
                ))),
            }
        })
        .collect()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn empty_memo_value(field: &FieldDescriptor) -> Value {
    if field.length == 4 {
        Value::Number(0.into())
    } else {
        Value::String(String::new())
    }
}

fn encode_memo_pointer(field: &FieldDescriptor, block: u32) -> Result<Vec<u8>, DbfError> {
    if field.length == 4 {
        return Ok(block.to_le_bytes().to_vec());
    }
    let text = block.to_string();
    let length = usize::from(field.length);
    if text.len() > length {
        return Err(DbfError::Invalid(format!(
            "memo block number for {} exceeds field width {}",
            field.name, field.length
        )));
    }
    let mut pointer = vec![b' '; length];
    let start = length - text.len();
    pointer[start..].copy_from_slice(text.as_bytes());
    Ok(pointer)
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
        b'B' | b'G' | b'M' if length == 4 => Ok(value_u32(value, field)?.to_le_bytes().to_vec()),
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
    let encoded = match language_driver {
        0x01 => encode_codepage(&text, CP437_UPPER),
        0x02 => encode_codepage(&text, CP850_UPPER),
        0x1f | 0x22 | 0x23 | 0x40 | 0x64 | 0x87 => encode_codepage(&text, CP852_UPPER),
        0x26 | 0x65 => encode_codepage(&text, CP866_UPPER),
        0x03 | 0x57 => encode_windows_1252(&text),
        _ => Some(text.into_bytes()),
    };
    encoded.ok_or_else(|| {
        let code_page = match language_driver {
            0x01 => "CP437",
            0x02 => "CP850",
            0x1f | 0x22 | 0x23 | 0x40 | 0x64 | 0x87 => "CP852",
            0x26 | 0x65 => "CP866",
            0x03 | 0x57 => "Windows-1252",
            _ => "the declared code page",
        };
        DbfError::Invalid(format!(
            "value for {} contains a character outside {code_page}",
            field.name
        ))
    })
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

fn value_u32(value: &Value, field: &FieldDescriptor) -> Result<u32, DbfError> {
    match value {
        Value::Null => Ok(0),
        Value::Number(number) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| DbfError::Invalid(format!("field {} requires a uint32", field.name))),
        Value::String(text) => text
            .trim()
            .parse::<u32>()
            .map_err(|_| DbfError::Invalid(format!("field {} requires a uint32", field.name))),
        _ => Err(DbfError::Invalid(format!(
            "field {} requires a uint32",
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
        b'B' | b'G' | b'M' if bytes.len() == 4 => {
            Value::Number(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).into())
        }
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
    match language_driver {
        0x01 => decode_codepage(bytes, CP437_UPPER),
        0x02 => decode_codepage(bytes, CP850_UPPER),
        0x1f | 0x22 | 0x23 | 0x40 | 0x64 | 0x87 => decode_codepage(bytes, CP852_UPPER),
        0x26 | 0x65 => decode_codepage(bytes, CP866_UPPER),
        0x03 | 0x57 => decode_windows_1252(bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn decode_codepage(bytes: &[u8], upper: &str) -> String {
    bytes
        .iter()
        .map(|byte| {
            if *byte < 0x80 {
                char::from(*byte)
            } else {
                upper
                    .chars()
                    .nth(usize::from(*byte - 0x80))
                    .unwrap_or('\u{fffd}')
            }
        })
        .collect()
}

fn encode_codepage(text: &str, upper: &str) -> Option<Vec<u8>> {
    text.chars()
        .map(|character| {
            if character <= '\u{7f}' {
                u8::try_from(u32::from(character)).ok()
            } else {
                upper
                    .chars()
                    .position(|candidate| candidate == character)
                    .and_then(|index| u8::try_from(index + 0x80).ok())
            }
        })
        .collect()
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
    fn decodes_and_encodes_oem_character_fields() {
        let mut bytes = fixture();
        let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
        let name_start = record_start + 4;

        bytes[29] = 0x01;
        bytes[name_start..name_start + 10].fill(b' ');
        bytes[name_start] = 0x82;
        let mut table = DbfTable::from_bytes(&bytes).unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], "é");
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "é"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        assert_eq!(table.to_bytes()[name_start], 0x82);

        bytes[29] = 0x02;
        bytes[name_start..name_start + 10].fill(b' ');
        bytes[name_start] = 0x9b;
        let mut table = DbfTable::from_bytes(&bytes).unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], "ø");
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "ø"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        assert_eq!(table.to_bytes()[name_start], 0x9b);

        bytes[29] = 0x64;
        bytes[name_start..name_start + 10].fill(b' ');
        bytes[name_start] = 0x88;
        let mut table = DbfTable::from_bytes(&bytes).unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], "ł");
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "ł"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        assert_eq!(table.to_bytes()[name_start], 0x88);

        bytes[29] = 0x65;
        bytes[name_start..name_start + 10].fill(b' ');
        bytes[name_start] = 0x9f;
        let mut table = DbfTable::from_bytes(&bytes).unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], "Я");
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "Я"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        assert_eq!(table.to_bytes()[name_start], 0x9f);

        let error = table
            .patch_record(
                1,
                serde_json::json!({"NAME": "漢"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("outside CP866"));
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
    fn applies_update_operators_without_ambiguous_writes() {
        let mut table = DbfTable::from_bytes(&fixture()).unwrap();
        table
            .patch_record(
                1,
                serde_json::json!({
                    "$set": {"NAME": "Alicia"},
                    "$inc": {"AGE": 1},
                    "$unset": {"ACTIVE": true}
                })
                .as_object()
                .unwrap()
                .clone(),
            )
            .unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], "Alicia");
        assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);
        assert_eq!(
            table.active_record(1).unwrap().values["ACTIVE"],
            Value::Null
        );

        let before = table.to_bytes();
        let error = table
            .patch_record(
                1,
                serde_json::json!({"$set": {"AGE": 31}, "$inc": {"AGE": 1}})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("multiple update operators"));
        assert_eq!(table.to_bytes(), before);

        let error = table
            .patch_record(
                1,
                serde_json::json!({"$unknown": {"AGE": 31}})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("unsupported update operator"));
    }

    #[test]
    fn reads_and_writes_dbase3_memo_sidecar() {
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
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "new memo"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        table
            .patch_record(
                1,
                serde_json::json!({"AGE": 30}).as_object().unwrap().clone(),
            )
            .unwrap();
        table.save_with_wal(&path).unwrap();

        let reread = DbfTable::from_path(&path).unwrap();
        assert_eq!(reread.active_record(1).unwrap().values["NAME"], "new memo");
        assert_eq!(reread.active_record(1).unwrap().values["AGE"], 30);
        assert_eq!(
            &reread.to_bytes()[memo_start..memo_start + 10],
            b"         2"
        );
        let memo = MemoFile::open(&memo_path, 0x83).unwrap();
        assert_eq!(memo.read(2).unwrap().unwrap(), b"new memo");
        assert_eq!(u32::from_be_bytes(memo.bytes[..4].try_into().unwrap()), 3);

        fs::remove_file(path).unwrap();
        fs::remove_file(memo_path).unwrap();
    }

    #[test]
    fn reads_and_writes_dbase4_memo_sidecar() {
        let path =
            std::env::temp_dir().join(format!("txbase-dbase4-memo-{}.dbf", std::process::id()));
        let memo_path = path.with_extension("dbt");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&memo_path);

        let mut bytes = fixture();
        bytes[0] = 0x8b;
        bytes[64 + 11] = b'M';
        let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
        let memo_start = record_start + 4;
        bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
        fs::write(&path, bytes).unwrap();

        let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
        let text = b"memo from dbase4";
        memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&[0xff, 0xff, 0x08, 0x00]);
        memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
            .copy_from_slice(&((text.len() as u32 + 8).to_le_bytes()));
        memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + text.len()].copy_from_slice(text);
        fs::write(&memo_path, memo).unwrap();

        let mut table = DbfTable::from_path(&path).unwrap();
        assert_eq!(
            table.active_record(1).unwrap().values["NAME"],
            "memo from dbase4"
        );
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "changed dbase4"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        table.save_with_wal(&path).unwrap();

        let reread = DbfTable::from_path(&path).unwrap();
        assert_eq!(
            reread.active_record(1).unwrap().values["NAME"],
            "changed dbase4"
        );
        assert_eq!(
            &reread.to_bytes()[memo_start..memo_start + 10],
            b"         2"
        );
        let memo = MemoFile::open(&memo_path, 0x8b).unwrap();
        assert_eq!(memo.read(2).unwrap().unwrap(), b"changed dbase4");
        assert_eq!(
            &memo.bytes[DBT_BLOCK_SIZE * 2..DBT_BLOCK_SIZE * 2 + 4],
            &[0xff, 0xff, 0x08, 0x00]
        );

        fs::remove_file(path).unwrap();
        fs::remove_file(memo_path).unwrap();
    }

    #[test]
    fn reads_and_writes_dbase4_binary_sidecar() {
        let path =
            std::env::temp_dir().join(format!("txbase-dbase4-binary-{}.dbf", std::process::id()));
        let memo_path = path.with_extension("dbt");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&memo_path);

        let mut bytes = fixture();
        bytes[0] = 0x8b;
        bytes[64 + 11] = b'B';
        let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
        let binary_start = record_start + 4;
        bytes[binary_start..binary_start + 10].copy_from_slice(b"         1");
        fs::write(&path, bytes).unwrap();

        let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
        let binary = [0x00, 0x1a, 0xff, 0x7f];
        memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&[0xff, 0xff, 0x08, 0x00]);
        memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
            .copy_from_slice(&((binary.len() as u32 + 8).to_le_bytes()));
        memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + binary.len()].copy_from_slice(&binary);
        fs::write(&memo_path, memo).unwrap();

        let mut table = DbfTable::from_path(&path).unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], "001aff7f");
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "deadbeef"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        table.save_with_wal(&path).unwrap();

        let reread = DbfTable::from_path(&path).unwrap();
        assert_eq!(reread.active_record(1).unwrap().values["NAME"], "deadbeef");
        assert_eq!(
            &reread.to_bytes()[binary_start..binary_start + 10],
            b"         2"
        );
        let memo = MemoFile::open(&memo_path, 0x8b).unwrap();
        assert_eq!(memo.read(2).unwrap().unwrap(), [0xde, 0xad, 0xbe, 0xef]);

        fs::remove_file(path).unwrap();
        fs::remove_file(memo_path).unwrap();
    }

    #[test]
    fn reads_and_writes_foxpro_fpt_memo_sidecar() {
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

        let mut table = DbfTable::from_path(&path).unwrap();
        assert_eq!(
            table.active_record(1).unwrap().values["NAME"],
            "memo from fpt"
        );
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "changed fpt"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        table.save_with_wal(&path).unwrap();

        let reread = DbfTable::from_path(&path).unwrap();
        assert_eq!(
            reread.active_record(1).unwrap().values["NAME"],
            "changed fpt"
        );
        assert_eq!(
            &reread.to_bytes()[memo_start..memo_start + 10],
            b"         2"
        );
        let memo = MemoFile::open(&memo_path, 0xf5).unwrap();
        assert_eq!(memo.read(2).unwrap().unwrap(), b"changed fpt");

        fs::remove_file(path).unwrap();
        fs::remove_file(memo_path).unwrap();
    }

    #[test]
    fn reads_and_writes_binary_fpt_sidecar_as_hex() {
        let path =
            std::env::temp_dir().join(format!("txbase-foxpro-binary-{}.dbf", std::process::id()));
        let memo_path = path.with_extension("fpt");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&memo_path);

        let mut bytes = fixture();
        bytes[0] = 0xf5;
        bytes[64 + 11] = b'B';
        let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
        let binary_start = record_start + 4;
        bytes[binary_start..binary_start + 10].copy_from_slice(b"         1");
        fs::write(&path, bytes).unwrap();

        let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
        memo[6..8].copy_from_slice(&(DBT_BLOCK_SIZE as u16).to_be_bytes());
        memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&2u32.to_be_bytes());
        let binary = [0x00, 0x01, 0xff, 0x7f];
        memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
            .copy_from_slice(&(binary.len() as u32).to_be_bytes());
        memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + binary.len()].copy_from_slice(&binary);
        fs::write(&memo_path, memo).unwrap();

        let mut table = DbfTable::from_path(&path).unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], "0001ff7f");
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "deadbeef"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        table.save_with_wal(&path).unwrap();

        let mut reread = DbfTable::from_path(&path).unwrap();
        assert_eq!(reread.active_record(1).unwrap().values["NAME"], "deadbeef");
        assert_eq!(
            &reread.to_bytes()[binary_start..binary_start + 10],
            b"         2"
        );
        let memo = MemoFile::open(&memo_path, 0xf5).unwrap();
        assert_eq!(memo.read(2).unwrap().unwrap(), [0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(
            &memo.bytes[DBT_BLOCK_SIZE * 2..DBT_BLOCK_SIZE * 2 + 4],
            &[0; 4]
        );

        table
            .patch_record(
                1,
                serde_json::json!({"AGE": 30}).as_object().unwrap().clone(),
            )
            .unwrap();
        table.save_with_wal(&path).unwrap();

        reread = DbfTable::from_path(&path).unwrap();
        assert_eq!(reread.active_record(1).unwrap().values["NAME"], "deadbeef");
        assert_eq!(reread.active_record(1).unwrap().values["AGE"], 30);
        assert_eq!(
            &reread.to_bytes()[binary_start..binary_start + 10],
            b"         2"
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

    #[test]
    fn recovers_dbf_and_memo_from_txdm_snapshot() {
        let path = std::env::temp_dir().join(format!(
            "txbase-dbf-memo-recovery-{}-{}.dbf",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let memo_path = path.with_extension("dbt");
        let wal_path = path.with_extension("txbase.wal");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&memo_path);
        let _ = fs::remove_file(&wal_path);

        let mut bytes = fixture();
        bytes[0] = 0x83;
        bytes[64 + 11] = b'M';
        let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
        let memo_start = record_start + 4;
        bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
        fs::write(&path, &bytes).unwrap();

        let mut memo_bytes = vec![0; DBT_BLOCK_SIZE * 2];
        let text = b"memo before crash";
        memo_bytes[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + text.len()].copy_from_slice(text);
        memo_bytes[DBT_BLOCK_SIZE + text.len()] = EOF_MARKER;
        fs::write(&memo_path, memo_bytes).unwrap();

        let mut table = DbfTable::from_path(&path).unwrap();
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": "memo after crash"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        let mut prepared = table.clone();
        let memo = prepared.apply_memo_updates(&path).unwrap().unwrap();
        let payload = memo_snapshot_payload(&prepared.to_bytes(), &memo).unwrap();
        let mut wal = FileWal::open(&wal_path).unwrap();
        wal.append(&payload).unwrap();
        wal.sync().unwrap();
        drop(wal);

        let recovered = DbfTable::from_path(&path).unwrap();
        assert_eq!(
            recovered.active_record(1).unwrap().values["NAME"],
            "memo after crash"
        );
        assert_eq!(
            &recovered.to_bytes()[memo_start..memo_start + 10],
            b"         2"
        );
        assert!(!wal_path.exists());

        fs::remove_file(path).unwrap();
        fs::remove_file(memo_path).unwrap();
    }
}
