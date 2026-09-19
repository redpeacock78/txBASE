use std::env;
use std::error::Error;
use std::io::{self, Write};
use std::path::PathBuf;
use txbase::{
    catalog::Catalog,
    dbf::DbfTable,
    dbf::copy_table_files,
    index::{IndexDefinition, IndexFile},
    server,
    xbf::XbfTable,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let Some(first) = args.next() else {
        print_help();
        return Ok(());
    };

    if first == "--help" || first == "-h" {
        print_help();
        return Ok(());
    }

    if first == "--serve" {
        let path = PathBuf::from(args.next().ok_or("--serve requires a DBF path")?);
        let mut bind = String::from("127.0.0.1:8080");
        let mut encoding = None;
        while let Some(option) = args.next() {
            match option.as_str() {
                "--bind" => bind = args.next().ok_or("--bind requires an address")?,
                "--encoding" => {
                    encoding = Some(args.next().ok_or("--encoding requires a name")?);
                }
                _ => return Err(format!("unknown option: {option}").into()),
            }
        }
        let table = DbfTable::from_path_with_encoding(&path, encoding.as_deref())?;
        return server::serve(table, &path, &bind).map_err(Into::into);
    }

    if first == "--serve-catalog" {
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
        return server::serve_catalog(&path, &bind).map_err(Into::into);
    }

    if first == "schema" || first == "verify" {
        let path = PathBuf::from(
            args.next()
                .ok_or_else(|| format!("{first} requires a DBF path"))?,
        );
        let encoding = parse_encoding_option(&mut args)?;
        let table = DbfTable::from_path_with_encoding(&path, encoding.as_deref())?;
        table.verify()?;
        let output = if first == "schema" {
            table.schema_json()
        } else {
            serde_json::json!({"valid": true, "schema": table.schema_json()})
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if first == "catalog" || first == "verify-catalog" {
        let path = PathBuf::from(
            args.next()
                .ok_or_else(|| format!("{first} requires a directory path"))?,
        );
        if let Some(extra) = args.next() {
            return Err(format!("unexpected argument: {extra}").into());
        }
        let catalog = Catalog::from_path(&path)?;
        if first == "verify-catalog" {
            catalog.verify()?;
            println!("{}", serde_json::json!({"valid": true}));
        } else {
            println!("{}", serde_json::to_string_pretty(&catalog.schema_json()?)?);
        }
        return Ok(());
    }

    if first == "xbf" {
        let operation = args
            .next()
            .ok_or_else(|| "xbf requires import or export".to_owned())?;
        let source = PathBuf::from(
            args.next()
                .ok_or_else(|| format!("xbf {operation} requires a source path"))?,
        );
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
        return Ok(());
    }

    if first == "index" {
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
        return Ok(());
    }

    if first == "pack" {
        let path = PathBuf::from(
            args.next()
                .ok_or_else(|| "pack requires a DBF path".to_owned())?,
        );
        let encoding = parse_encoding_option(&mut args)?;
        let mut table = DbfTable::from_path_with_encoding(&path, encoding.as_deref())?;
        table.pack()?;
        table.save_with_wal(&path)?;
        println!("pack: {}", path.display());
        return Ok(());
    }

    if first == "recall" {
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
        return Ok(());
    }

    if first == "backup" || first == "restore" {
        let source = PathBuf::from(
            args.next()
                .ok_or_else(|| format!("{first} requires a source DBF path"))?,
        );
        let destination = PathBuf::from(
            args.next()
                .ok_or_else(|| format!("{first} requires a destination DBF path"))?,
        );
        if let Some(extra) = args.next() {
            return Err(format!("unexpected argument: {extra}").into());
        }
        copy_table_files(&source, &destination)?;
        println!(
            "{}: {} -> {}",
            first,
            source.display(),
            destination.display()
        );
        return Ok(());
    }

    if first.starts_with('-') {
        return Err(format!("unknown option: {first}").into());
    }
    let encoding = parse_encoding_option(&mut args)?;

    let table = DbfTable::from_path_with_encoding(first, encoding.as_deref())?;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, &table.active_json())?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

fn print_help() {
    println!(
        "Usage:\n  txbase FILE [--encoding NAME]\n  txbase schema FILE [--encoding NAME]\n  txbase verify FILE [--encoding NAME]\n  txbase catalog DIRECTORY\n  txbase verify-catalog DIRECTORY\n  txbase xbf import DBF XBF [--encoding NAME]\n  txbase xbf export XBF DBF [--schema]\n  txbase index build FILE FIELD...\n  txbase index build-compound FILE NAME FIELD[:1|-1] FIELD[:1|-1]...\n  txbase index verify FILE\n  txbase index rebuild FILE\n  txbase pack FILE [--encoding NAME]\n  txbase recall FILE RECORD [--encoding NAME]\n  txbase backup SOURCE DEST\n  txbase restore SOURCE DEST\n  txbase --serve FILE [--bind ADDRESS] [--encoding NAME]\n  txbase --serve-catalog DIRECTORY [--bind ADDRESS]\n\nReads active DBF records as JSON. NAME accepts the supported CJK aliases and takes precedence over a schema sidecar override for that invocation. Schema, catalog, verification, and index commands inspect DBF files. xbf import writes a bounded XBF snapshot from a DBF; xbf export writes a representable XBF table as DBF, and --schema also writes its constraint sidecar. Backup and restore copy a DBF with its sibling memo sidecar. The single-table server exposes GET /records, GET /records/{{id}}, QUERY /records, and JSON mutations. The catalog server exposes GET /catalog and QUERY /join for bounded read-only joins."
    );
}

fn parse_encoding_option(
    args: &mut impl Iterator<Item = String>,
) -> Result<Option<String>, Box<dyn Error>> {
    let Some(option) = args.next() else {
        return Ok(None);
    };
    if option != "--encoding" {
        return Err(format!("unexpected argument: {option}").into());
    }
    let name = args.next().ok_or("--encoding requires a name")?;
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument: {extra}").into());
    }
    Ok(Some(name))
}

fn parse_compound_field(specification: &str) -> Result<(String, i8), Box<dyn Error>> {
    let Some((field, raw_direction)) = specification.split_once(':') else {
        return Ok((specification.to_owned(), 1));
    };
    if field.is_empty() {
        return Err("compound index field name is empty".into());
    }
    let direction = match raw_direction.to_ascii_lowercase().as_str() {
        "1" | "asc" => 1,
        "-1" | "desc" => -1,
        _ => {
            return Err(format!(
                "compound index direction must be 1, -1, asc, or desc: {raw_direction}"
            )
            .into());
        }
    };
    Ok((field.to_owned(), direction))
}
