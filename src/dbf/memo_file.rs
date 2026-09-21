use super::super::{DBT_BLOCK_SIZE, DbfError, EOF_MARKER, MemoFile, MemoFormat};
use std::fs;
use std::path::Path;

impl MemoFile {
    pub(crate) fn open(path: &Path, dbf_version: u8) -> Result<Self, DbfError> {
        let bytes = fs::read(path)?;
        let format = if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("fpt"))
        {
            MemoFormat::FoxPro
        } else {
            if dbf_version == 0x83 {
                MemoFormat::Dbase3
            } else {
                MemoFormat::Dbase4
            }
        };
        Self::from_bytes(bytes, format)
    }

    pub(crate) fn from_bytes(bytes: Vec<u8>, format: MemoFormat) -> Result<Self, DbfError> {
        let block_size = match format {
            MemoFormat::FoxPro => usize::from(u16::from_be_bytes(
                bytes
                    .get(6..8)
                    .ok_or_else(|| DbfError::Invalid("FPT header is truncated".into()))?
                    .try_into()
                    .expect("FPT block size is fixed"),
            )),
            MemoFormat::Dbase3 => {
                if bytes.len() < DBT_BLOCK_SIZE {
                    return Err(DbfError::Invalid("DBT header is truncated".into()));
                }
                DBT_BLOCK_SIZE
            }
            MemoFormat::Dbase4 => {
                if bytes.len() < DBT_BLOCK_SIZE {
                    return Err(DbfError::Invalid("DBT header is truncated".into()));
                }
                let block_size = usize::from(u16::from_le_bytes([bytes[20], bytes[21]]));
                if block_size == 0 {
                    DBT_BLOCK_SIZE
                } else {
                    block_size
                }
            }
        };
        if block_size < 8 || bytes.len() < block_size {
            return Err(DbfError::Invalid(
                "memo block size or header is invalid".into(),
            ));
        }
        if format != MemoFormat::FoxPro
            && (block_size < DBT_BLOCK_SIZE || block_size % DBT_BLOCK_SIZE != 0)
        {
            return Err(DbfError::Invalid("dBASE DBT block size is invalid".into()));
        }
        Ok(Self {
            bytes,
            block_size,
            format,
        })
    }

    pub(crate) fn read(&self, block: u32) -> Result<Option<Vec<u8>>, DbfError> {
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
                    .windows(2)
                    .position(|pair| pair == [EOF_MARKER, EOF_MARKER])
                    .or_else(|| {
                        data.iter()
                            .position(|byte| *byte == EOF_MARKER)
                            .filter(|&index| {
                                data[index + 1..]
                                    .iter()
                                    .all(|byte| matches!(*byte, 0 | b' ' | EOF_MARKER))
                            })
                    })
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

    pub(crate) fn append_text(&mut self, data: &[u8]) -> Result<u32, DbfError> {
        let mut payload = Vec::new();
        match self.format {
            MemoFormat::Dbase3 => {
                if data.contains(&EOF_MARKER) {
                    return Err(DbfError::Invalid(
                        "dBASE III memo text contains the end marker".into(),
                    ));
                }
                payload.extend_from_slice(data);
                payload.extend_from_slice(&[EOF_MARKER, EOF_MARKER]);
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

    pub(crate) fn append_binary(&mut self, data: &[u8]) -> Result<u32, DbfError> {
        let length = u32::try_from(data.len())
            .map_err(|_| DbfError::Invalid("binary data is too long".into()))?;
        let mut payload = Vec::with_capacity(8usize.saturating_add(data.len()));
        match self.format {
            MemoFormat::Dbase3 => {
                // ponytail: dBASE III has no binary length; reserve 0x1a1a as its
                // terminator and reject payloads that cannot round-trip through it.
                if data.last() == Some(&EOF_MARKER)
                    || data.windows(2).any(|pair| pair == [EOF_MARKER, EOF_MARKER])
                {
                    return Err(DbfError::Invalid(
                        "dBASE III binary data cannot contain its 0x1a1a terminator".into(),
                    ));
                }
            }
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
        }
        payload.extend_from_slice(data);
        if self.format == MemoFormat::Dbase3 {
            payload.extend_from_slice(&[EOF_MARKER, EOF_MARKER]);
        }
        self.append_payload(payload)
    }

    pub(crate) fn append_payload(&mut self, mut payload: Vec<u8>) -> Result<u32, DbfError> {
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
        let next_block_bytes = if self.format == MemoFormat::Dbase4 {
            next_block.to_le_bytes()
        } else {
            next_block.to_be_bytes()
        };
        self.bytes[0..4].copy_from_slice(&next_block_bytes);
        u32::try_from(start_block)
            .map_err(|_| DbfError::Invalid("memo block number overflows u32".into()))
    }
}
