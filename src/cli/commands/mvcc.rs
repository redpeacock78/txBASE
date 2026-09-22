use std::error::Error;
use std::path::PathBuf;

use serde_json::json;
use txbase::catalog::Catalog;
use txbase::dbf::{DbfTable, RowId};

pub(crate) fn mvcc(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let action = args
        .next()
        .ok_or("mvcc requires list, read, row, row-at, or gc")?;
    match action.as_str() {
        "list" => {
            let path = PathBuf::from(args.next().ok_or("mvcc list requires a DBF path")?);
            reject_extra(args)?;
            println!(
                "{}",
                serde_json::to_string(&DbfTable::mvcc_versions(path)?)?
            );
        }
        "read" => {
            let path = PathBuf::from(args.next().ok_or("mvcc read requires a DBF path")?);
            let transaction_id = parse_positive_u64(
                args.next().ok_or("mvcc read requires a transaction ID")?,
                "mvcc transaction ID",
            )?;
            reject_extra(args)?;
            let table = DbfTable::from_path_at(path, transaction_id)?;
            println!("{}", serde_json::to_string(&table.active_json())?);
        }
        "row" => {
            let path = PathBuf::from(args.next().ok_or("mvcc row requires a DBF path")?);
            let record_number = parse_positive_usize(
                args.next().ok_or("mvcc row requires a record number")?,
                "mvcc row record number",
            )?;
            reject_extra(args)?;
            println!(
                "{}",
                serde_json::to_string(&DbfTable::mvcc_row_versions(path, record_number)?)?
            );
        }
        "row-at" => {
            let path = PathBuf::from(args.next().ok_or("mvcc row-at requires a DBF path")?);
            let transaction_id = parse_positive_u64(
                args.next().ok_or("mvcc row-at requires a transaction ID")?,
                "mvcc row-at transaction ID",
            )?;
            let epoch = parse_positive_u64(
                args.next().ok_or("mvcc row-at requires an epoch")?,
                "mvcc row-at epoch",
            )?;
            let record_number = parse_positive_usize(
                args.next().ok_or("mvcc row-at requires a record number")?,
                "mvcc row-at record number",
            )?;
            reject_extra(args)?;
            println!(
                "{}",
                serde_json::to_string(&DbfTable::mvcc_read_row(
                    path,
                    transaction_id,
                    RowId {
                        epoch,
                        record_number,
                    },
                )?)?
            );
        }
        "gc" => {
            let path = PathBuf::from(args.next().ok_or("mvcc gc requires a DBF path")?);
            let (keep_last, keep_rows) = parse_table_gc_options(&mut args)?;
            let retained = match keep_rows {
                Some(keep_rows) => {
                    DbfTable::gc_mvcc_with_row_retention(path, keep_last, keep_rows)?
                }
                None => DbfTable::gc_mvcc(path, keep_last)?,
            };
            println!("{}", serde_json::to_string(&retained)?);
        }
        "catalog" => catalog_mvcc(args)?,
        _ => return Err(format!("unknown mvcc command: {action}").into()),
    }
    Ok(())
}

fn catalog_mvcc(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let action = args.next().ok_or("mvcc catalog requires list or read")?;
    match action.as_str() {
        "list" => {
            let path = PathBuf::from(
                args.next()
                    .ok_or("mvcc catalog list requires a directory")?,
            );
            reject_extra(args)?;
            println!("{}", serde_json::to_string(&Catalog::mvcc_versions(path)?)?);
        }
        "read" => {
            let path = PathBuf::from(
                args.next()
                    .ok_or("mvcc catalog read requires a directory")?,
            );
            let transaction_id = parse_positive_u64(
                args.next()
                    .ok_or("mvcc catalog read requires a transaction ID")?,
                "mvcc catalog transaction ID",
            )?;
            reject_extra(args)?;
            let catalog = Catalog::from_path_at(&path, transaction_id)?;
            let tables = catalog
                .tables()
                .map(|table| {
                    let records = catalog.open_table(table.name())?.active_json();
                    Ok(json!({"name": table.name(), "records": records}))
                })
                .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "format": "txbase-catalog-snapshot",
                    "transaction_id": transaction_id,
                    "tables": tables,
                }))?
            );
        }
        "gc" => {
            let path = PathBuf::from(args.next().ok_or("mvcc catalog gc requires a directory")?);
            let keep_last = parse_keep_last(&mut args, "mvcc catalog gc")?;
            println!(
                "{}",
                serde_json::to_string(&Catalog::gc_mvcc(path, keep_last)?)?
            );
        }
        _ => return Err(format!("unknown mvcc catalog command: {action}").into()),
    }
    Ok(())
}

fn reject_extra(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument: {extra}").into());
    }
    Ok(())
}

fn parse_keep_last(
    args: &mut impl Iterator<Item = String>,
    command: &str,
) -> Result<usize, Box<dyn Error>> {
    if args.next().as_deref() != Some("--keep") {
        return Err(format!("{command} requires --keep COUNT").into());
    }
    let value = args
        .next()
        .ok_or_else(|| format!("{command} requires a keep count"))?;
    let keep_last = value
        .parse::<usize>()
        .map_err(|_| format!("{command} keep count must be a positive integer"))?;
    if keep_last == 0 {
        return Err(format!("{command} keep count must be a positive integer").into());
    }
    reject_extra(args)?;
    Ok(keep_last)
}

fn parse_table_gc_options(
    args: &mut impl Iterator<Item = String>,
) -> Result<(usize, Option<usize>), Box<dyn Error>> {
    let mut keep_last = None;
    let mut keep_rows = None;
    while let Some(option) = args.next() {
        let target = match option.as_str() {
            "--keep" => &mut keep_last,
            "--keep-rows" => &mut keep_rows,
            _ => return Err(format!("unexpected argument: {option}").into()),
        };
        if target.is_some() {
            return Err(format!("{option} may only be specified once").into());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("{option} requires a positive integer"))?;
        let parsed = value
            .parse::<usize>()
            .map_err(|_| format!("{option} requires a positive integer"))?;
        if parsed == 0 {
            return Err(format!("{option} requires a positive integer").into());
        }
        *target = Some(parsed);
    }
    let keep_last = keep_last.ok_or("mvcc gc requires --keep COUNT")?;
    Ok((keep_last, keep_rows))
}

fn parse_positive_u64(value: String, label: &str) -> Result<u64, Box<dyn Error>> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{label} must be a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{label} must be a positive integer").into());
    }
    Ok(parsed)
}

fn parse_positive_usize(value: String, label: &str) -> Result<usize, Box<dyn Error>> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{label} must be a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{label} must be a positive integer").into());
    }
    Ok(parsed)
}
