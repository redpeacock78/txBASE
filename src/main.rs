use std::env;
use std::error::Error;
use std::io::{self, Write};
use std::path::PathBuf;
use txbase::{dbf::DbfTable, server};

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
        return server::serve(DbfTable::from_path(path)?, &bind).map_err(Into::into);
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
        "Usage:\n  txbase FILE\n  txbase --serve FILE [--bind ADDRESS]\n\nReads active DBF records as JSON. The server exposes GET /records, GET /records/{{id}}, and executes QUERY /records."
    );
}
