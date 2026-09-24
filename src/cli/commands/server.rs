use std::error::Error;
use std::path::PathBuf;

use txbase::{dbf::DbfTable, server};

pub(crate) fn serve(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(args.next().ok_or("serve requires a DBF path")?);
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

pub(crate) fn serve_catalog(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(
        args.next()
            .ok_or("serve-catalog requires a directory path")?,
    );
    let mut bind = String::from("127.0.0.1:8080");
    let mut replication_term = 1;
    while let Some(option) = args.next() {
        match option.as_str() {
            "--bind" => bind = args.next().ok_or("--bind requires an address")?,
            "--replication-term" => {
                replication_term = args
                    .next()
                    .ok_or("--replication-term requires a positive integer")?
                    .parse::<u64>()?;
                if replication_term == 0 {
                    return Err("--replication-term must be positive".into());
                }
            }
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }
    server::serve_catalog_with_replication_term(&path, &bind, replication_term).map_err(Into::into)
}
