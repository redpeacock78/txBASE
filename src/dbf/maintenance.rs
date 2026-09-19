use super::schema_metadata::schema_metadata_path;
use super::{
    ACTIVE_RECORD, DbfError, DbfTable, EOF_MARKER, find_memo_path, sync_parent_directory,
    write_record_count,
};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn copy_table_files(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<(), DbfError> {
    let source = source.as_ref();
    let destination = destination.as_ref();
    if source == destination {
        return Err(DbfError::Invalid(
            "source and destination must be different paths".into(),
        ));
    }

    let table = DbfTable::from_path(source)?;
    table.verify()?;
    let dbf_bytes = fs::read(source)?;
    let source_memo = find_memo_path(source);
    let memo_bytes = source_memo.as_ref().map(fs::read).transpose()?;
    let source_schema = schema_metadata_path(source);
    let schema_bytes = match fs::read(&source_schema) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let destination_memo = source_memo.as_ref().and_then(|path| {
        path.extension()
            .map(|extension| destination.with_extension(extension))
    });
    let destination_schema = schema_metadata_path(destination);

    let dbf_temp = write_temp(destination, &dbf_bytes, "dbf")?;
    let memo_temp = match (&destination_memo, &memo_bytes) {
        (Some(path), Some(bytes)) => Some(write_temp(path, bytes, "memo")?),
        _ => None,
    };
    let schema_temp = schema_bytes
        .as_ref()
        .map(|bytes| write_temp(&destination_schema, bytes, "schema"))
        .transpose()?;

    if let Err(error) = replace_file(&dbf_temp, destination) {
        let _ = fs::remove_file(&dbf_temp);
        if let Some(path) = memo_temp {
            let _ = fs::remove_file(path);
        }
        if let Some(path) = schema_temp {
            let _ = fs::remove_file(path);
        }
        return Err(error.into());
    }

    remove_other_memo_sidecars(destination, destination_memo.as_deref())?;
    if let (Some(temp), Some(path)) = (memo_temp, destination_memo.as_ref()) {
        replace_file(&temp, path)?;
    }
    if let Some(temp) = schema_temp {
        replace_file(&temp, &destination_schema)?;
    } else {
        remove_file_if_exists(&destination_schema)?;
    }
    sync_parent_directory(destination)?;
    if let Some(path) = destination_memo.as_ref() {
        sync_parent_directory(path)?;
    }
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<(), std::io::Error> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

impl DbfTable {
    pub fn recall_record(&mut self, number: usize) -> Result<(), DbfError> {
        let index = number
            .checked_sub(1)
            .ok_or_else(|| DbfError::Invalid("record id must be a positive integer".into()))?;
        let Some(record) = self.records.get(index) else {
            return Err(DbfError::Invalid("record not found".into()));
        };
        if !record.deleted {
            return Err(DbfError::Invalid("record is not deleted".into()));
        }
        if let Some(schema) = &self.schema {
            schema.validate_candidate(&record.values, &self.records, Some(index))?;
        }
        let offset = self.record_offset(index)?;
        let mut bytes = self.bytes.clone();
        bytes[offset] = ACTIVE_RECORD;
        self.bytes = bytes;
        self.records[index].deleted = false;
        Ok(())
    }

    pub fn pack(&mut self) -> Result<(), DbfError> {
        if !self.memo_updates.is_empty() {
            return Err(DbfError::Invalid(
                "memo updates must be saved before PACK".into(),
            ));
        }
        let record_end = self.record_end_for_pack()?;
        let suffix = &self.bytes[record_end..];
        if !suffix.is_empty() && suffix != [EOF_MARKER] {
            return Err(DbfError::Invalid(
                "cannot PACK a DBF with trailing sidecar data".into(),
            ));
        }

        let record_length = usize::from(self.header.record_length);
        let header_length = usize::from(self.header.header_length);
        let mut bytes = self.bytes[..header_length].to_vec();
        let mut records = Vec::new();
        let mut stored_values = Vec::new();
        for (index, record) in self.records.iter().enumerate() {
            if record.deleted {
                continue;
            }
            let offset = header_length + index * record_length;
            bytes.extend_from_slice(&self.bytes[offset..offset + record_length]);
            let mut record = record.clone();
            record.number = records.len() + 1;
            records.push(record);
            stored_values.push(self.stored_values[index].clone());
        }
        let count = u32::try_from(records.len())
            .map_err(|_| DbfError::Invalid("record count exceeds DBF limit".into()))?;
        write_record_count(&mut bytes, count)?;
        bytes.push(EOF_MARKER);

        self.bytes = bytes;
        self.header.record_count = count;
        self.records = records;
        self.stored_values = stored_values;
        Ok(())
    }

    fn record_end_for_pack(&self) -> Result<usize, DbfError> {
        let record_bytes = self
            .records
            .len()
            .checked_mul(usize::from(self.header.record_length))
            .ok_or_else(|| DbfError::Invalid("record area overflows usize".into()))?;
        usize::from(self.header.header_length)
            .checked_add(record_bytes)
            .ok_or_else(|| DbfError::Invalid("record area overflows usize".into()))
            .and_then(|end| {
                (end <= self.bytes.len())
                    .then_some(end)
                    .ok_or_else(|| DbfError::Invalid("record area is truncated".into()))
            })
    }
}

fn write_temp(path: &Path, bytes: &[u8], kind: &str) -> Result<PathBuf, DbfError> {
    let temporary = temporary_path(path, kind);
    let mut file = File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(temporary)
}

fn temporary_path(path: &Path, kind: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(format!(".txbase-{kind}-{}.tmp", std::process::id()));
    PathBuf::from(value)
}

fn replace_file(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)
}

fn remove_other_memo_sidecars(
    destination: &Path,
    retained: Option<&Path>,
) -> Result<(), std::io::Error> {
    for extension in ["dbt", "DBT", "fpt", "FPT"] {
        let candidate = destination.with_extension(extension);
        if retained.is_some_and(|path| path == candidate.as_path()) {
            continue;
        }
        match fs::remove_file(candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
