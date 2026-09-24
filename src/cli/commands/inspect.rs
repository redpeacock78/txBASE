use std::error::Error;
use std::fs;
use std::path::PathBuf;

use txbase::{
    Collation,
    catalog::Catalog,
    dbf::DbfTable,
    index::{IndexDefinition, IndexFile},
    transaction::FileWal,
};

use super::super::{parse_collation, parse_compound_field, parse_encoding_option};

pub(crate) fn inspect(
    command: &str,
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(
        args.next()
            .ok_or_else(|| format!("{command} requires a DBF path"))?,
    );
    inspect_path(command, path, args)
}

pub(crate) fn schema(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let first = args.next().ok_or("schema requires a DBF path or apply")?;
    if first == "apply" {
        let path = PathBuf::from(args.next().ok_or("schema apply requires a DBF path")?);
        let metadata_path = PathBuf::from(
            args.next()
                .ok_or("schema apply requires a schema JSON path")?,
        );
        if let Some(extra) = args.next() {
            return Err(format!("unexpected argument: {extra}").into());
        }
        let schema_bytes = fs::read(&metadata_path)?;
        txbase::dbf::apply_schema_metadata(&path, &schema_bytes)?;
        println!(
            "schema apply: {} <- {}",
            path.display(),
            metadata_path.display()
        );
        return Ok(());
    }
    inspect_path("schema", PathBuf::from(first), args)
}

fn inspect_path(
    command: &str,
    path: PathBuf,
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
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
            let (fields, collation) = parse_index_options(args)?;
            let definitions = fields
                .into_iter()
                .map(|field| {
                    let definition = IndexDefinition::for_field(field);
                    collation.map_or(definition.clone(), |value| definition.with_collation(value))
                })
                .collect::<Vec<_>>();
            if definitions.is_empty() {
                return Err("index build requires at least one field".into());
            }
            let index = IndexFile::build(&path, definitions)?;
            index.save(&path)?;
            println!("{}", serde_json::to_string_pretty(&index.schema_json())?);
        }
        "build-compound" => {
            let (index_args, collation) = parse_index_options(args)?;
            let mut index_args = index_args.into_iter();
            let name = index_args
                .next()
                .ok_or_else(|| "index build-compound requires an index name".to_owned())?;
            let mut fields = Vec::new();
            let mut directions = Vec::new();
            for specification in index_args {
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
            let definition = if let Some(value) = collation {
                definition.with_collation(value)
            } else {
                definition
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

fn parse_index_options(
    args: impl Iterator<Item = String>,
) -> Result<(Vec<String>, Option<Collation>), Box<dyn Error>> {
    let mut positional = Vec::new();
    let mut collation = None;
    let mut args = args;
    while let Some(argument) = args.next() {
        if argument == "--collation" {
            if collation.is_some() {
                return Err("index collation was specified more than once".into());
            }
            let value = args.next().ok_or("--collation requires a name")?;
            collation = Some(parse_collation(&value)?);
        } else if argument.starts_with('-') {
            return Err(format!("unexpected argument: {argument}").into());
        } else {
            positional.push(argument);
        }
    }
    Ok((positional, collation))
}
