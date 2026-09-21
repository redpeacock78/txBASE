use std::error::Error;
use std::io::{self, Write};
use std::path::PathBuf;

use txbase::{dbf::DbfTable, dbf::copy_table_files, xbf::XbfTable};

use super::super::parse_encoding_option;

pub(crate) fn xbf(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
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

pub(crate) fn pack(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
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

pub(crate) fn recall(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
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

pub(crate) fn copy_files(
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

pub(crate) fn read(
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
