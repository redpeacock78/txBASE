use std::error::Error;
use std::io::{self, Write};
use std::path::PathBuf;

use txbase::{
    catalog::Catalog,
    dbf::DbfTable,
    dbf::copy_table_files,
    index::{IndexDefinition, IndexFile},
    server,
    transaction::FileWal,
    xbf::XbfTable,
};

use super::{parse_compound_field, parse_encoding_option};

pub(super) fn serve(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(args.next().ok_or("--serve requires a DBF path")?);
    let mut bind = String::from("127.0.0.1:8080");
    let mut encoding = None;
    while let Some(option) = args.next() {
        match option.as_str() {
            "--bind" => bind = args.next().ok_or("--bind requires an address")?,
            "--encoding" => encoding = Some(args.next().ok_or("--encoding requires a name")?),
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }
    let table = DbfTable::from_path_with_encoding(&path, encoding.as_deref())?;
    server::serve(table, &path, &bind).map_err(Into::into)
}

pub(super) fn serve_catalog(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(
        args.next()
            .ok_or("--serve-catalog requires a directory path")?,
    );
    let mut bind = String::from("127.0.0.1:8080");
    while let Some(option) = args.next() {
        match option.as_str() {
            "--bind" => bind = args.next().ok_or("--bind requires an address")?,
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }
    server::serve_catalog(&path, &bind).map_err(Into::into)
}

pub(super) fn inspect(
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

pub(super) fn catalog(
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

pub(super) fn xbf(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let operation = args
        .next()
        .ok_or_else(|| "xbf requires import, export, or report".to_owned())?;
    let source = PathBuf::from(
        args.next()
            .ok_or_else(|| format!("xbf {operation} requires a source path"))?,
    );
    if operation == "report" {
        if let Some(extra) = args.next() {
            return Err(format!("unexpected argument: {extra}").into());
        }
        let table = XbfTable::from_path(&source)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&table.dbf_export_report())?
        );
        return Ok(());
    }
    let destination = PathBuf::from(
        args.next()
            .ok_or_else(|| format!("xbf {operation} requires a destination path"))?,
    );
    match operation.as_str() {
        "import" => {
            let encoding = parse_encoding_option(&mut args)?;
            let dbf = DbfTable::from_path_with_encoding(&source, encoding.as_deref())?;
            XbfTable::from_dbf(&dbf)?.save_to(&destination)?;
        }
        "export" => {
            let preserve_schema = match args.next() {
                None => false,
                Some(option) if option == "--schema" => true,
                Some(extra) => return Err(format!("unexpected argument: {extra}").into()),
            };
            let table = XbfTable::from_path(&source)?;
            if preserve_schema {
                table.save_dbf_with_schema(&destination)?;
            } else {
                table.to_dbf()?.save_to(&destination)?;
            }
        }
        _ => return Err(format!("unknown xbf operation: {operation}").into()),
    }
    println!(
        "xbf {operation}: {} -> {}",
        source.display(),
        destination.display()
    );
    Ok(())
}

pub(super) fn wal(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
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

pub(super) fn index(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
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

pub(super) fn pack(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(
        args.next()
            .ok_or_else(|| "pack requires a DBF path".to_owned())?,
    );
    let encoding = parse_encoding_option(&mut args)?;
    let mut table = DbfTable::from_path_with_encoding(&path, encoding.as_deref())?;
    table.pack()?;
    table.save_with_wal(&path)?;
    println!("pack: {}", path.display());
    Ok(())
}

pub(super) fn recall(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(
        args.next()
            .ok_or_else(|| "recall requires a DBF path".to_owned())?,
    );
    let raw_id = args
        .next()
        .ok_or_else(|| "recall requires a record number".to_owned())?;
    let id = raw_id
        .parse::<usize>()
        .map_err(|_| format!("invalid record number: {raw_id}"))?;
    let encoding = parse_encoding_option(&mut args)?;
    let mut table = DbfTable::from_path_with_encoding(&path, encoding.as_deref())?;
    table.recall_record(id)?;
    table.save_with_wal(&path)?;
    println!("recall: {} record {}", path.display(), id);
    Ok(())
}

pub(super) fn copy_files(
    command: &str,
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
    let source = PathBuf::from(
        args.next()
            .ok_or_else(|| format!("{command} requires a source DBF path"))?,
    );
    let destination = PathBuf::from(
        args.next()
            .ok_or_else(|| format!("{command} requires a destination DBF path"))?,
    );
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument: {extra}").into());
    }
    copy_table_files(&source, &destination)?;
    println!(
        "{command}: {} -> {}",
        source.display(),
        destination.display()
    );
    Ok(())
}

pub(super) fn read(
    path: &str,
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
    let encoding = parse_encoding_option(&mut args)?;
    let table = DbfTable::from_path_with_encoding(path, encoding.as_deref())?;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, &table.active_json())?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}
