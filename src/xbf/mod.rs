mod checksum;
mod codec;
mod conversion;
mod export;
mod persistence;
mod query;
mod schema;
mod values;
mod wal;

pub(crate) use checksum::crc32c;
pub(crate) use query::record_values;

#[cfg(test)]
mod export_tests;
#[cfg(test)]
mod malformed_tests;
#[cfg(test)]
mod tests;

use std::error::Error;
use std::fmt::{self, Display, Formatter};

pub const HEADER_SIZE: usize = 100;
pub const MAGIC: [u8; 4] = *b"TXBF";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XbfType {
    Boolean,
    Signed32,
    Signed64,
    Unsigned64,
    Float32,
    Float64,
    String,
    Bytes,
    Date,
    Timestamp,
    Uuid,
    Json,
}

impl XbfType {
    pub(crate) fn tag(self) -> u8 {
        match self {
            Self::Boolean => 0x01,
            Self::Signed32 => 0x10,
            Self::Signed64 => 0x11,
            Self::Unsigned64 => 0x12,
            Self::Float32 => 0x20,
            Self::Float64 => 0x21,
            Self::String => 0x30,
            Self::Bytes => 0x31,
            Self::Date => 0x40,
            Self::Timestamp => 0x41,
            Self::Uuid => 0x60,
            Self::Json => 0x70,
        }
    }

    pub(crate) fn from_tag(tag: u8) -> Result<Self, XbfError> {
        match tag {
            0x01 => Ok(Self::Boolean),
            0x10 => Ok(Self::Signed32),
            0x11 => Ok(Self::Signed64),
            0x12 => Ok(Self::Unsigned64),
            0x20 => Ok(Self::Float32),
            0x21 => Ok(Self::Float64),
            0x30 => Ok(Self::String),
            0x31 => Ok(Self::Bytes),
            0x40 => Ok(Self::Date),
            0x41 => Ok(Self::Timestamp),
            0x60 => Ok(Self::Uuid),
            0x70 => Ok(Self::Json),
            0x50 => Err(XbfError::Invalid(
                "decimal type is reserved in XBF v1".into(),
            )),
            _ => Err(XbfError::Invalid(format!(
                "unsupported XBF type tag {tag:#04x}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XbfField {
    pub name: String,
    pub ty: XbfType,
    pub nullable: bool,
    pub primary_key: bool,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum XbfValue {
    Null,
    Boolean(bool),
    Signed32(i32),
    Signed64(i64),
    Unsigned64(u64),
    Float32(f32),
    Float64(f64),
    String(String),
    Bytes(Vec<u8>),
    Date(i32),
    Timestamp(i64),
    Uuid([u8; 16]),
    Json(serde_json::Value),
}

#[derive(Debug, Clone, PartialEq)]
pub struct XbfRecord {
    pub deleted: bool,
    pub values: Vec<XbfValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct XbfTable {
    pub generation: u64,
    pub fields: Vec<XbfField>,
    pub records: Vec<XbfRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XbfLimits {
    pub max_file_size: usize,
    pub max_section_size: usize,
    pub max_record_size: usize,
    pub max_value_size: usize,
    pub max_field_name: usize,
    pub max_fields: usize,
    pub max_records: usize,
}

impl Default for XbfLimits {
    fn default() -> Self {
        Self {
            max_file_size: 256 * 1024 * 1024,
            max_section_size: 256 * 1024 * 1024,
            max_record_size: 16 * 1024 * 1024,
            max_value_size: 16 * 1024 * 1024,
            max_field_name: 255,
            max_fields: 4_096,
            max_records: 1_000_000,
        }
    }
}

#[derive(Debug)]
pub enum XbfError {
    Io(std::io::Error),
    Invalid(String),
}

impl Display for XbfError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid XBF: {message}"),
        }
    }
}

impl Error for XbfError {}

impl From<std::io::Error> for XbfError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub use codec::{decode, decode_with_limits, encode, encode_with_limits};
pub use conversion::from_dbf;
pub use export::{
    XbfExportIssue, XbfExportReport, dbf_export_report, save_dbf_with_schema, to_dbf,
    to_dbf_with_schema,
};
pub use persistence::{read_path, read_path_with_limits, write_path, write_path_with_limits};
pub use wal::{recover_path, save_with_wal};

impl XbfTable {
    pub fn from_dbf(table: &crate::dbf::DbfTable) -> Result<Self, XbfError> {
        conversion::from_dbf(table)
    }

    pub fn to_dbf(&self) -> Result<crate::dbf::DbfTable, XbfError> {
        export::to_dbf(self)
    }

    pub fn dbf_export_report(&self) -> export::XbfExportReport {
        export::dbf_export_report(self)
    }

    pub fn to_dbf_with_schema(
        &self,
    ) -> Result<(crate::dbf::DbfTable, serde_json::Value), XbfError> {
        export::to_dbf_with_schema(self)
    }

    pub fn save_dbf_with_schema(&self, path: impl AsRef<std::path::Path>) -> Result<(), XbfError> {
        export::save_dbf_with_schema(self, path)
    }

    pub fn from_path(path: impl AsRef<std::path::Path>) -> Result<Self, XbfError> {
        read_path(path)
    }

    pub fn from_path_with_limits(
        path: impl AsRef<std::path::Path>,
        limits: &XbfLimits,
    ) -> Result<Self, XbfError> {
        read_path_with_limits(path, limits)
    }

    pub fn save_to(&self, path: impl AsRef<std::path::Path>) -> Result<(), XbfError> {
        write_path(path, self)
    }

    pub fn save_to_with_limits(
        &self,
        path: impl AsRef<std::path::Path>,
        limits: &XbfLimits,
    ) -> Result<(), XbfError> {
        write_path_with_limits(path, self, limits)
    }

    pub fn save_with_wal(&self, path: impl AsRef<std::path::Path>) -> Result<(), XbfError> {
        save_with_wal(path, self)
    }
}
