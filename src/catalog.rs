use crate::dbf::{DbfError, DbfTable};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum CatalogError {
    Io(std::io::Error),
    Invalid(String),
    Table { name: String, source: DbfError },
}

impl Display for CatalogError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "catalog I/O error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid catalog: {message}"),
            Self::Table { name, source } => write!(formatter, "table {name}: {source}"),
        }
    }
}

impl Error for CatalogError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Table { source, .. } => Some(source),
            Self::Invalid(_) => None,
        }
    }
}

impl From<std::io::Error> for CatalogError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogTable {
    name: String,
    file_name: String,
    path: PathBuf,
}

impl CatalogTable {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    root: PathBuf,
    tables: BTreeMap<String, CatalogTable>,
}

impl Catalog {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, CatalogError> {
        let root = path.as_ref().to_path_buf();
        if !fs::metadata(&root)?.is_dir() {
            return Err(CatalogError::Invalid(format!(
                "catalog path is not a directory: {}",
                root.display()
            )));
        }

        let mut tables = BTreeMap::new();
        for entry in fs::read_dir(&root)? {
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

        Ok(Self { root, tables })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn table_names(&self) -> Vec<String> {
        self.tables.keys().cloned().collect()
    }

    pub fn tables(&self) -> impl Iterator<Item = &CatalogTable> {
        self.tables.values()
    }

    pub fn table_path(&self, name: &str) -> Option<&Path> {
        self.tables.get(name).map(CatalogTable::path)
    }

    pub fn open_table(&self, name: &str) -> Result<DbfTable, CatalogError> {
        let table = self
            .tables
            .get(name)
            .ok_or_else(|| CatalogError::Invalid(format!("table not found: {name}")))?;
        DbfTable::from_path(table.path()).map_err(|source| CatalogError::Table {
            name: table.name.clone(),
            source,
        })
    }

    pub fn schema_json(&self) -> Result<Value, CatalogError> {
        let mut tables = Vec::with_capacity(self.tables.len());
        for entry in self.tables() {
            let table = self.open_table(entry.name())?;
            tables.push(json!({
                "name": entry.name(),
                "file": entry.file_name(),
                "schema": table.schema_json(),
            }));
        }
        Ok(json!({
            "format": "txbase-catalog",
            "tables": tables,
        }))
    }

    pub fn verify(&self) -> Result<(), CatalogError> {
        for entry in self.tables() {
            let table = self.open_table(entry.name())?;
            table.verify().map_err(|source| CatalogError::Table {
                name: entry.name().to_owned(),
                source,
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_CATALOG_ID: AtomicUsize = AtomicUsize::new(0);

    fn fixture() -> Vec<u8> {
        include_str!("../tests/fixtures/users.dbf.hex")
            .split_whitespace()
            .map(|token| u8::from_str_radix(token, 16).unwrap())
            .collect()
    }

    fn temporary_catalog() -> PathBuf {
        let id = NEXT_CATALOG_ID.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("txbase-catalog-test-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn discovers_direct_dbf_tables_and_ignores_other_files() {
        let root = temporary_catalog();
        fs::write(root.join("users.dbf"), fixture()).unwrap();
        fs::write(root.join("posts.DBF"), fixture()).unwrap();
        fs::write(root.join("users.dbt"), b"memo").unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested").join("ignored.dbf"), fixture()).unwrap();

        let catalog = Catalog::from_path(&root).unwrap();

        assert_eq!(catalog.table_names(), vec!["posts", "users"]);
        assert_eq!(
            catalog.table_path("users").unwrap(),
            root.join("users.dbf").as_path()
        );
        assert_eq!(catalog.open_table("users").unwrap().active_json().len(), 1);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn schema_and_verify_cover_every_discovered_table() {
        let root = temporary_catalog();
        fs::write(root.join("users.dbf"), fixture()).unwrap();
        fs::write(root.join("posts.dbf"), fixture()).unwrap();

        let catalog = Catalog::from_path(&root).unwrap();
        catalog.verify().unwrap();
        let schema = catalog.schema_json().unwrap();

        assert_eq!(schema["format"], "txbase-catalog");
        assert_eq!(schema["tables"].as_array().unwrap().len(), 2);
        assert_eq!(schema["tables"][0]["name"], "posts");
        assert_eq!(schema["tables"][1]["schema"]["format"], "dbf");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verify_reports_a_malformed_table_by_name() {
        let root = temporary_catalog();
        fs::write(root.join("broken.dbf"), b"not a DBF").unwrap();

        let error = Catalog::from_path(&root).unwrap().verify().unwrap_err();

        assert!(error.to_string().contains("table broken"));
        fs::remove_dir_all(root).unwrap();
    }
}
