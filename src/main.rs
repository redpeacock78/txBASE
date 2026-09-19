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
        while let Some(option) = args.next() {
            if option != "--bind" {
                return Err(format!("unknown option: {option}").into());
            }
            bind = args.next().ok_or("--bind requires an address")?;
        }
        let table = DbfTable::from_path(&path)?;
        return server::serve(table, &path, &bind).map_err(Into::into);
    }

    if first == "schema" || first == "verify" {
        let path = PathBuf::from(
            args.next()
                .ok_or_else(|| format!("{first} requires a DBF path"))?,
        );
        if let Some(extra) = args.next() {
            return Err(format!("unexpected argument: {extra}").into());
        }
        let table = DbfTable::from_path(&path)?;
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
                let fields = args.collect::<Vec<_>>();
                if fields.len() < 2 {
                    return Err("index build-compound requires at least two fields".into());
                }
                let index =
                    IndexFile::build(&path, vec![IndexDefinition::named_fields(name, fields)])?;
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
        if let Some(extra) = args.next() {
            return Err(format!("unexpected argument: {extra}").into());
        }
        let mut table = DbfTable::from_path(&path)?;
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
        if let Some(extra) = args.next() {
            return Err(format!("unexpected argument: {extra}").into());
        }
        let mut table = DbfTable::from_path(&path)?;
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
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument: {extra}").into());
    }

    let table = DbfTable::from_path(first)?;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, &table.active_json())?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

fn print_help() {
    println!(
        "Usage:\n  txbase FILE\n  txbase schema FILE\n  txbase verify FILE\n  txbase catalog DIRECTORY\n  txbase verify-catalog DIRECTORY\n  txbase index build FILE FIELD...\n  txbase index build-compound FILE NAME FIELD FIELD...\n  txbase index verify FILE\n  txbase index rebuild FILE\n  txbase pack FILE\n  txbase recall FILE RECORD\n  txbase backup SOURCE DEST\n  txbase restore SOURCE DEST\n  txbase --serve FILE [--bind ADDRESS]\n\nReads active DBF records as JSON. Schema, catalog, verification, and index commands inspect DBF files. Backup and restore copy a DBF with its sibling memo sidecar. The server exposes GET /records, GET /records/{{id}}, executes QUERY /records, and persists JSON mutations."
    );
}
