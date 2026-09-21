use std::error::Error;
use std::path::PathBuf;

use txbase::dbf::{DbfFieldSpec, DbfTable};

pub(crate) fn init(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(args.next().ok_or("init requires a destination DBF path")?);
    if path.exists() {
        return Err(format!("refusing to overwrite existing DBF: {}", path.display()).into());
    }

    let mut fields = Vec::new();
    while let Some(option) = args.next() {
        if option != "--field" {
            return Err(format!("unexpected argument: {option}").into());
        }
        let specification = args
            .next()
            .ok_or("--field requires NAME:TYPE:LENGTH[:DECIMALS]")?;
        fields.push(parse_field(&specification)?);
    }
    let table = DbfTable::empty(&fields)?;
    table.save_to(&path)?;
    println!("initialized: {}", path.display());
    Ok(())
}

pub(crate) fn insert(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(args.next().ok_or("insert requires a DBF path")?);
    let raw_record = args.next().ok_or("insert requires a JSON object")?;
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument: {extra}").into());
    }
    let record = serde_json::from_str::<serde_json::Value>(&raw_record)?
        .as_object()
        .cloned()
        .ok_or("insert JSON must be an object")?;
    let mut table = DbfTable::from_path(&path)?;
    let number = table.insert_record(record)?;
    table.save_with_wal(&path)?;
    println!("inserted: {} record {}", path.display(), number);
    Ok(())
}

fn parse_field(specification: &str) -> Result<DbfFieldSpec, Box<dyn Error>> {
    let parts = specification.split(':').collect::<Vec<_>>();
    if !(3..=4).contains(&parts.len()) {
        return Err(format!("field must be NAME:TYPE:LENGTH[:DECIMALS]: {specification}").into());
    }
    let field_type = parts[1].as_bytes();
    if field_type.len() != 1 {
        return Err(format!("field type must be one ASCII byte: {}", parts[1]).into());
    }
    let length = parts[2]
        .parse::<u8>()
        .map_err(|_| format!("field length is invalid: {}", parts[2]))?;
    let decimal_count = parts.get(3).map_or(Ok(0), |value| {
        value
            .parse::<u8>()
            .map_err(|_| format!("field decimal count is invalid: {value}"))
    })?;
    Ok(DbfFieldSpec::new(
        parts[0],
        field_type[0].to_ascii_uppercase(),
        length,
        decimal_count,
    ))
}
