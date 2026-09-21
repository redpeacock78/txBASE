use super::{CatalogError, CatalogTable};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub(super) fn discover_tables(root: &Path) -> Result<BTreeMap<String, CatalogTable>, CatalogError> {
    let mut tables = BTreeMap::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }

        let path = entry.path();
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !extension.eq_ignore_ascii_case("dbf") {
            continue;
        }

        let Some(name) = path
            .file_stem()
            .and_then(|value| value.to_str())
            .map(ToOwned::to_owned)
        else {
            return Err(CatalogError::Invalid(format!(
                "DBF filename is not valid UTF-8: {}",
                path.display()
            )));
        };
        if name.is_empty() {
            return Err(CatalogError::Invalid(format!(
                "DBF filename has an empty table name: {}",
                path.display()
            )));
        }
        if tables
            .insert(
                name.to_owned(),
                CatalogTable {
                    name: name.clone(),
                    file_name: entry.file_name().to_string_lossy().into_owned(),
                    path,
                },
            )
            .is_some()
        {
            return Err(CatalogError::Invalid(format!(
                "duplicate table name: {name}"
            )));
        }
    }
    Ok(tables)
}
