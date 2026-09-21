use std::error::Error;
use std::path::PathBuf;

use serde_json::json;
use txbase::catalog::Catalog;
use txbase::dbf::DbfTable;

pub(crate) fn mvcc(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let action = args.next().ok_or("mvcc requires list or read")?;
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
            let transaction_id = args
                .next()
                .ok_or("mvcc read requires a transaction ID")?
                .parse::<u64>()
                .map_err(|_| "mvcc transaction ID must be a positive integer")?;
            reject_extra(args)?;
            let table = DbfTable::from_path_at(path, transaction_id)?;
            println!("{}", serde_json::to_string(&table.active_json())?);
        }
        "gc" => {
            let path = PathBuf::from(args.next().ok_or("mvcc gc requires a DBF path")?);
            let keep_last = parse_keep_last(&mut args, "mvcc gc")?;
            println!(
                "{}",
                serde_json::to_string(&DbfTable::gc_mvcc(path, keep_last)?)?
            );
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
            let transaction_id = args
                .next()
                .ok_or("mvcc catalog read requires a transaction ID")?
                .parse::<u64>()
                .map_err(|_| "mvcc transaction ID must be a positive integer")?;
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
