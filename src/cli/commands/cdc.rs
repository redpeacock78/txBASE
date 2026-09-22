use std::error::Error;
use std::path::PathBuf;

use txbase::dbf::DbfTable;

pub(crate) fn cdc(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(args.next().ok_or("cdc requires a DBF path")?);
    let after = match args.next().as_deref() {
        None => None,
        Some("--after") => Some(parse_positive_u64(
            args.next().ok_or("cdc --after requires a transaction ID")?,
        )?),
        Some(option) => return Err(format!("unexpected argument: {option}").into()),
    };
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument: {extra}").into());
    }
    println!(
        "{}",
        serde_json::to_string(&DbfTable::cdc_events(&path, after)?)?
    );
    Ok(())
}

fn parse_positive_u64(value: String) -> Result<u64, Box<dyn Error>> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| "cdc transaction ID must be a positive integer")?;
    if parsed == 0 {
        return Err("cdc transaction ID must be a positive integer".into());
    }
    Ok(parsed)
}
