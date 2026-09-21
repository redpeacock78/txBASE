use std::error::Error;
use std::path::PathBuf;

use txbase::{
    catalog::Catalog,
    dbf::DbfTable,
    index::{IndexDefinition, IndexFile},
    transaction::FileWal,
};

use super::super::{parse_compound_field, parse_encoding_option};

pub(crate) fn inspect(
    command: &str,
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(
        args.next()
            .ok_or_else(|| format!("{command} requires a DBF path"))?,
    );
    let encoding = parse_encoding_option(&mut args)?;
    let table = DbfTable::from_path_with_encoding(&path, encoding.as_deref())?;
    table.verify()?;
    if command == "verify" && txbase::index::sidecar_path(&path).exists() {
        IndexFile::load(&path).map_err(|error| format!("index sidecar is invalid: {error}"))?;
    }
    let output = if command == "schema" {
        table.schema_json()
    } else {
        serde_json::json!({"valid": true, "schema": table.schema_json()})
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub(crate) fn catalog(
    command: &str,
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(
        args.next()
            .ok_or_else(|| format!("{command} requires a directory path"))?,
    );
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument: {extra}").into());
    }
    let catalog = Catalog::from_path(&path)?;
    if command == "verify-catalog" {
        catalog.verify()?;
        println!("{}", serde_json::json!({"valid": true}));
    } else {
        println!("{}", serde_json::to_string_pretty(&catalog.schema_json()?)?);
    }
    Ok(())
}

pub(crate) fn wal(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let operation = args
        .next()
        .ok_or_else(|| "wal requires inspect".to_owned())?;
    if operation != "inspect" {
        return Err(format!("unknown wal operation: {operation}").into());
    }
    let path = PathBuf::from(
        args.next()
            .ok_or_else(|| "wal inspect requires a WAL path".to_owned())?,
    );
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument: {extra}").into());
    }
    let inspection = FileWal::inspect(&path)?;
    let output = serde_json::json!({
        "format": "txbase-wal",
        "path": path.display().to_string(),
        "file_bytes": inspection.file_bytes,
        "valid_bytes": inspection.valid_bytes,
        "truncated_tail": inspection.truncated_tail,
        "records": inspection.records.iter().map(|record| serde_json::json!({
            "lsn": record.lsn.0,
            "length": record.length,
        })).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub(crate) fn index(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let operation = args
        .next()
        .ok_or_else(|| "index requires build, build-compound, verify, or rebuild".to_owned())?;
    let path = PathBuf::from(
        args.next()
            .ok_or_else(|| format!("index {operation} requires a DBF path"))?,
    );
    match operation.as_str() {
        "build" => {
            let definitions = args.map(IndexDefinition::for_field).collect::<Vec<_>>();
            if definitions.is_empty() {
                return Err("index build requires at least one field".into());
            }
            let index = IndexFile::build(&path, definitions)?;
            index.save(&path)?;
            println!("{}", serde_json::to_string_pretty(&index.schema_json())?);
        }
        "build-compound" => {
            let name = args
                .next()
                .ok_or_else(|| "index build-compound requires an index name".to_owned())?;
            let mut fields = Vec::new();
            let mut directions = Vec::new();
            for specification in args {
                let (field, direction) = parse_compound_field(&specification)?;
                fields.push(field);
                directions.push(direction);
            }
            if fields.len() < 2 {
                return Err("index build-compound requires at least two fields".into());
            }
            let definition = if directions.iter().all(|direction| *direction == 1) {
                IndexDefinition::named_fields(name, fields)
            } else {
                IndexDefinition::named_fields_with_directions(name, fields, directions)
            };
            let index = IndexFile::build(&path, vec![definition])?;
            index.save(&path)?;
            println!("{}", serde_json::to_string_pretty(&index.schema_json())?);
        }
        "verify" => {
            if let Some(extra) = args.next() {
                return Err(format!("unexpected argument: {extra}").into());
            }
            let index = IndexFile::load(&path)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "valid": true,
                    "schema": index.schema_json(),
                }))?
            );
        }
        "rebuild" => {
            if let Some(extra) = args.next() {
                return Err(format!("unexpected argument: {extra}").into());
            }
            let index = IndexFile::rebuild(&path)?;
            println!("{}", serde_json::to_string_pretty(&index.schema_json())?);
        }
        _ => return Err(format!("unknown index operation: {operation}").into()),
    }
    Ok(())
}
