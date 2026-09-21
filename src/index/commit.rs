use super::{INDEX_FORMAT, INDEX_VERSION, IndexError, IndexFile, sidecar_path};
use crate::dbf::DbfTable;
use std::path::Path;

const INDEX_SNAPSHOT_MAGIC: &[u8; 4] = b"TXDI";

pub(crate) fn pending_snapshot_payload(
    dbf_path: &Path,
    table: &DbfTable,
    dbf_bytes: &[u8],
    memo_bytes: Option<&[u8]>,
) -> Result<Option<Vec<u8>>, IndexError> {
    let Some(index) = refreshed_index(dbf_path, table, dbf_bytes, memo_bytes)? else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec_pretty(&index)?;
    let mut payload = INDEX_SNAPSHOT_MAGIC.to_vec();
    payload.extend_from_slice(&bytes);
    Ok(Some(payload))
}

pub(crate) fn snapshot_bytes_without_memo(
    dbf_path: &Path,
    table: &DbfTable,
    dbf_bytes: &[u8],
) -> Result<Option<Vec<u8>>, IndexError> {
    let Some(index) = refreshed_index_without_memo(dbf_path, table, dbf_bytes)? else {
        return Ok(None);
    };
    Ok(Some(serde_json::to_vec_pretty(&index)?))
}

pub(crate) fn validate_snapshot_bytes(
    dbf_path: &Path,
    dbf_bytes: &[u8],
    index_bytes: &[u8],
) -> Result<(), IndexError> {
    let index = serde_json::from_slice::<IndexFile>(index_bytes)?;
    let table = DbfTable::from_bytes(dbf_bytes)?;
    super::validation::validate_for_table(&index, &table)?;
    if index.source != super::storage::source_fingerprint_for_without_memo(dbf_bytes) {
        return Err(IndexError::Stale {
            path: sidecar_path(dbf_path),
        });
    }
    Ok(())
}

pub(crate) fn refresh_if_present(dbf_path: &Path, table: &DbfTable) -> Result<(), IndexError> {
    let Some(index) = refreshed_index(dbf_path, table, &table.to_bytes(), None)? else {
        return Ok(());
    };
    let bytes = serde_json::to_vec_pretty(&index)?;
    let index_path = sidecar_path(dbf_path);
    super::storage::write_atomic(&index_path, &bytes)
}

pub(crate) fn decode_snapshot_payload(payload: &[u8]) -> Result<Option<Vec<u8>>, IndexError> {
    let Some(bytes) = payload.strip_prefix(INDEX_SNAPSHOT_MAGIC) else {
        return Ok(None);
    };
    let index = serde_json::from_slice::<IndexFile>(bytes)?;
    super::validation::validate_shape(&index)?;
    Ok(Some(bytes.to_vec()))
}

pub(crate) fn apply_snapshot_payload(dbf_path: &Path, payload: &[u8]) -> Result<(), IndexError> {
    let Some(bytes) = decode_snapshot_payload(payload)? else {
        return Ok(());
    };
    let index = serde_json::from_slice::<IndexFile>(&bytes)?;
    let index_path = sidecar_path(dbf_path);
    if index.source != super::storage::source_fingerprint(dbf_path)? {
        return Err(IndexError::Stale { path: index_path });
    }
    super::storage::write_atomic(&index_path, &bytes)
}

fn refreshed_index(
    dbf_path: &Path,
    table: &DbfTable,
    dbf_bytes: &[u8],
    memo_bytes: Option<&[u8]>,
) -> Result<Option<IndexFile>, IndexError> {
    let index_path = sidecar_path(dbf_path);
    if !index_path.is_file() {
        return Ok(None);
    }

    let existing = super::storage::read_sidecar(&index_path)?;
    let definitions = existing
        .indexes
        .iter()
        .map(|index| index.definition.clone())
        .collect::<Vec<_>>();
    let indexes = super::validation::build_indexes(table, &definitions)?;
    let source = super::storage::source_fingerprint_for(dbf_path, dbf_bytes, memo_bytes)?;
    let statistics = Some(super::statistics::build(
        table.active_records().count(),
        &indexes,
    ));
    if existing.source == source && existing.indexes == indexes {
        return Ok(None);
    }

    Ok(Some(IndexFile {
        format: INDEX_FORMAT.to_owned(),
        version: INDEX_VERSION,
        source,
        statistics,
        indexes,
    }))
}

fn refreshed_index_without_memo(
    dbf_path: &Path,
    table: &DbfTable,
    dbf_bytes: &[u8],
) -> Result<Option<IndexFile>, IndexError> {
    let index_path = sidecar_path(dbf_path);
    if !index_path.is_file() {
        return Ok(None);
    }

    let existing = super::storage::read_sidecar(&index_path)?;
    let definitions = existing
        .indexes
        .iter()
        .map(|index| index.definition.clone())
        .collect::<Vec<_>>();
    let indexes = super::validation::build_indexes(table, &definitions)?;
    let source = super::storage::source_fingerprint_for_without_memo(dbf_bytes);
    let statistics = Some(super::statistics::build(
        table.active_records().count(),
        &indexes,
    ));
    if existing.source == source && existing.indexes == indexes {
        return Ok(None);
    }

    Ok(Some(IndexFile {
        format: INDEX_FORMAT.to_owned(),
        version: INDEX_VERSION,
        source,
        statistics,
        indexes,
    }))
}
