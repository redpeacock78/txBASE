#[cfg(all(target_arch = "wasm32", target_os = "wasi", target_env = "p2"))]
mod component {
    use std::ffi::OsString;
    use std::future::poll_fn;
    use std::path::PathBuf;

    use txbase::dbf::DbfTable;
    use txbase::query::{self, AsyncQueryStream};

    wasip3::cli::command::export!(WasiQueryCommand);

    struct WasiQueryCommand;

    impl wasip3::exports::cli::run::Guest for WasiQueryCommand {
        async fn run() -> Result<(), ()> {
            match run_query().await {
                Ok(()) => Ok(()),
                Err(error) => {
                    eprintln!("{error}");
                    Err(())
                }
            }
        }
    }

    async fn run_query() -> Result<(), String> {
        let mut args = std::env::args_os().skip(1);
        let path = args
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| "usage: wasi-query-stream <dbf-path> <query-json>".to_owned())?;
        let query_json = args
            .next()
            .and_then(|value: OsString| value.into_string().ok())
            .ok_or_else(|| "query JSON must be valid UTF-8".to_owned())?;
        if args.next().is_some() {
            return Err("expected exactly a DBF path and query JSON".into());
        }

        let bytes = std::fs::read(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let table = DbfTable::from_bytes(&bytes).map_err(|error| error.to_string())?;
        let request = query::parse(query_json.as_bytes()).map_err(|error| error.to_string())?;
        let stream = query::stream_query(&table, &request).map_err(|error| error.to_string())?;
        let mut stream = Box::pin(stream);
        let (mut writer, reader) = wasip3::wit_stream::new::<u8>();

        let producer = async move {
            loop {
                let Some(item) = poll_fn(|context| stream.as_mut().poll_next(context)).await else {
                    return Ok::<(), String>(());
                };
                let row = item.map_err(|error| error.to_string())?;
                let mut bytes = serde_json::to_vec(&row).map_err(|error| error.to_string())?;
                bytes.push(b'\n');
                if !writer.write_all(bytes).await.is_empty() {
                    return Err("stdout closed while streaming query results".into());
                }
            }
        };
        let consumer = async {
            wasip3::cli::stdout::write_via_stream(reader)
                .await
                .map_err(|error| format!("stdout: {error:?}"))
        };

        futures::try_join!(producer, consumer).map(|_| ())
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "wasi", target_env = "p2")))]
#[allow(dead_code)]
fn main() {}
