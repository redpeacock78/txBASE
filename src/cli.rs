use std::env;
use std::error::Error;

mod commands;

pub(super) fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let Some(first) = args.next() else {
        print_help();
        return Ok(());
    };

    if first == "--help" || first == "-h" {
        print_help();
        return Ok(());
    }

    match first.as_str() {
        "read" => commands::read(args),
        "init" => commands::init(args),
        "insert" => commands::insert(args),
        "mvcc" => commands::mvcc(args),
        "serve" => commands::serve(args),
        "serve-catalog" => commands::serve_catalog(args),
        "schema" | "verify" => commands::inspect(&first, args),
        "catalog" | "verify-catalog" => commands::catalog(&first, args),
        "xbf" => commands::xbf(args),
        "wal" => commands::wal(args),
        "index" => commands::index(args),
        "pack" => commands::pack(args),
        "recall" => commands::recall(args),
        "backup" | "restore" => commands::copy_files(&first, args),
        _ if first.starts_with('-') => Err(format!("unknown option: {first}").into()),
        _ => Err(format!("unknown command: {first}").into()),
    }
}

fn print_help() {
    println!(
        "Usage:\n  txbase read FILE [--encoding NAME]\n  txbase init FILE --field NAME:TYPE:LENGTH[:DECIMALS]...\n  txbase insert FILE JSON_OBJECT\n  txbase mvcc list FILE\n  txbase mvcc read FILE TRANSACTION_ID\n  txbase schema FILE [--encoding NAME]\n  txbase verify FILE [--encoding NAME]\n  txbase catalog DIRECTORY\n  txbase verify-catalog DIRECTORY\n  txbase xbf import DBF XBF [--encoding NAME]\n  txbase xbf export XBF DBF [--schema]\n  txbase xbf report XBF\n  txbase index build FILE FIELD...\n  txbase index build-compound FILE NAME FIELD[:1|-1] FIELD[:1|-1]...\n  txbase index verify FILE\n  txbase index rebuild FILE\n  txbase pack FILE [--encoding NAME]\n  txbase recall FILE RECORD [--encoding NAME]\n  txbase backup SOURCE DEST\n  txbase restore SOURCE DEST\n  txbase serve FILE [--bind ADDRESS] [--encoding NAME]\n  txbase serve-catalog DIRECTORY [--bind ADDRESS]\n\nReads active DBF records as JSON. NAME accepts the supported CJK aliases and takes precedence over a schema sidecar override for that invocation. init creates a classic empty DBF from repeated field specifications, and insert appends one JSON object through the normal WAL path. mvcc list reports committed table snapshots, and mvcc read loads one historical snapshot. Schema, catalog, verification, and index commands inspect DBF files. xbf import writes a bounded XBF snapshot from a DBF; xbf export writes a representable XBF table as DBF, and --schema also writes its constraint sidecar; xbf report checks DBF representability without writing. Backup and restore copy a DBF with its detected memo, schema, transaction-state, MVCC, and valid index sidecars. The single-table server exposes GET /records, GET /records/{{id}}, QUERY /records, QUERY /explain, and JSON mutations. The catalog server exposes GET /catalog, GET/HEAD /{{table}}/records[/{{id}}], QUERY /{{table}}/records, QUERY /{{table}}/explain, and QUERY /join as bounded read-only routes."
    );
    println!(
        "\nAdditional command:\n  txbase wal inspect WAL\n\nwal inspect reads a WAL without creating or truncating it and reports complete record lengths plus an incomplete final tail."
    );
    println!(
        "\nCatalog MVCC commands:\n  txbase mvcc catalog list DIRECTORY\n  txbase mvcc catalog read DIRECTORY TRANSACTION_ID"
    );
}

pub(super) fn parse_encoding_option(
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

pub(super) fn parse_compound_field(specification: &str) -> Result<(String, i8), Box<dyn Error>> {
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
