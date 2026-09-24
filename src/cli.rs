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
        "cdc" => commands::cdc(args),
        "mvcc" => commands::mvcc(args),
        "serve" => commands::serve(args),
        "serve-catalog" => commands::serve_catalog(args),
        "schema" => commands::schema(args),
        "verify" => commands::inspect("verify", args),
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
        "Usage:\n  txbase read FILE [--encoding NAME]\n  txbase init FILE --field NAME:TYPE:LENGTH[:DECIMALS]...\n  txbase insert FILE JSON_OBJECT\n  txbase mvcc list FILE\n  txbase mvcc read FILE TRANSACTION_ID\n  txbase mvcc row FILE RECORD\n  txbase mvcc row-at FILE TRANSACTION_ID EPOCH RECORD\n  txbase schema FILE [--encoding NAME]\n  txbase schema apply FILE SCHEMA_JSON\n  txbase verify FILE [--encoding NAME]\n  txbase catalog DIRECTORY\n  txbase verify-catalog DIRECTORY\n  txbase xbf import DBF XBF [--encoding NAME]\n  txbase xbf export XBF DBF [--schema]\n  txbase xbf report XBF\n  txbase index build FILE FIELD...\n  txbase index build-compound FILE NAME FIELD[:DIRECTION] FIELD[:DIRECTION]...\n  txbase index verify FILE\n  txbase index rebuild FILE\n  txbase pack FILE [--encoding NAME]\n  txbase recall FILE RECORD [--encoding NAME]\n  txbase backup SOURCE DEST\n  txbase restore SOURCE DEST\n  txbase serve FILE [--bind ADDRESS] [--encoding NAME]\n  txbase serve-catalog DIRECTORY [--bind ADDRESS] [--replication-term TERM] [--replication-role authority|follower]\n\nReads active DBF records as JSON. NAME accepts the supported CJK aliases and takes precedence over a schema sidecar override for that invocation. init creates a classic empty DBF from repeated field specifications, and insert appends one JSON object through the normal WAL path. mvcc list reports committed table snapshots, mvcc read loads one historical snapshot, mvcc row lists retained versions for a physical record, and mvcc row-at reads one retained row version. Schema, catalog, verification, and index commands inspect DBF files. xbf import writes a bounded XBF snapshot from a DBF; xbf export writes a representable XBF table as DBF, and --schema also writes its constraint sidecar; xbf report checks DBF representability without writing. Backup and restore copy a DBF with its detected memo, schema, transaction-state, MVCC, and valid index sidecars. The single-table server exposes GET /records, GET /records/{{id}}, QUERY /records, QUERY /explain, and JSON mutations. The catalog server exposes GET /catalog, GET/HEAD /{{table}}/records[/{{id}}], QUERY /{{table}}/records, QUERY /{{table}}/explain, and QUERY /join as bounded read-only routes."
    );
    println!(
        "\nHTTP CDC: GET/HEAD /cdc supports exclusive after and bounded limit cursors on both server surfaces."
    );
    println!(
        "\nCatalog replication: txbase serve-catalog DIRECTORY [--bind ADDRESS] [--replication-term TERM] [--replication-role authority|follower] exposes GET /replication/status, GET /replication/snapshot, POST /replication/entry, and POST /replication/snapshot. Authority mode captures catalog mutations; follower mode rejects direct catalog mutations and accepts replication delivery."
    );
    println!(
        "\nCDC:\n  txbase cdc FILE [--after TRANSACTION_ID]\n  txbase cdc catalog DIRECTORY [--after TRANSACTION_ID]"
    );
    println!("\nSchema metadata:\n  txbase schema apply FILE SCHEMA_JSON");
    println!(
        "\nWAL command:\n  txbase wal inspect WAL\n\nwal inspect reads a WAL without creating or truncating it and reports complete record lengths plus an incomplete final tail."
    );
    println!(
        "\nCDC reads committed row-change events in transaction order; the catalog form reads atomic multi-table events; --after returns only later transaction IDs."
    );
    println!(
        "\nMVCC retention:\n  txbase mvcc gc FILE --keep COUNT [--keep-rows COUNT]\n\n--keep retains full snapshots; --keep-rows additionally retains older row versions per physical row."
    );
    println!(
        "\nCatalog MVCC commands:\n  txbase mvcc catalog list DIRECTORY\n  txbase mvcc catalog read DIRECTORY TRANSACTION_ID\n  txbase mvcc catalog gc DIRECTORY --keep COUNT"
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

#[cfg(test)]
mod tests {
    use super::{parse_compound_field, parse_encoding_option};

    fn arguments<'a>(values: &'a [&'a str]) -> impl Iterator<Item = String> + 'a {
        values.iter().map(|value| (*value).to_owned())
    }

    #[test]
    fn parses_encoding_option_and_rejects_malformed_shapes() {
        let mut args = arguments(&["--encoding", "gbk"]);
        assert_eq!(
            parse_encoding_option(&mut args).unwrap(),
            Some(String::from("gbk"))
        );
        assert!(parse_encoding_option(&mut arguments(&["--encoding"])).is_err());
        assert!(parse_encoding_option(&mut arguments(&["--wrong", "gbk"])).is_err());
        assert!(parse_encoding_option(&mut arguments(&["--encoding", "gbk", "extra"])).is_err());
    }

    #[test]
    fn parses_compound_field_directions_and_rejects_invalid_fields() {
        assert_eq!(
            parse_compound_field("NAME").unwrap(),
            (String::from("NAME"), 1)
        );
        assert_eq!(
            parse_compound_field("AGE:asc").unwrap(),
            (String::from("AGE"), 1)
        );
        assert_eq!(
            parse_compound_field("AGE:DESC").unwrap(),
            (String::from("AGE"), -1)
        );
        assert!(parse_compound_field(":1").is_err());
        assert!(parse_compound_field("AGE:sideways").is_err());
    }
}
