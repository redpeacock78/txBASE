#[cfg(all(target_arch = "wasm32", target_os = "wasi", target_env = "p2"))]
#[path = "wasi_query_stream/object_store.rs"]
mod object_store;

#[cfg(all(target_arch = "wasm32", target_os = "wasi", target_env = "p2"))]
mod component {
    use std::ffi::OsString;
    use std::future::poll_fn;
    use std::path::PathBuf;

    use txbase::edge::{AsyncObjectTable, SyncObjectStoreAdapter};
    use txbase::query::{self, AsyncQueryStream, QueryError};

    use crate::object_store::ReadOnlyFilesystemObjectStore;

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
        let first = args.next().ok_or_else(usage)?;

        if first == "--object-store" {
            let root = PathBuf::from(required_arg(&mut args, "object-store root")?);
            if root.as_os_str().is_empty() {
                return Err("object-store root must not be empty".into());
            }
            let namespace = required_utf8_arg(&mut args, "namespace")?;
            let query_json = required_utf8_arg(&mut args, "query JSON")?;
            let generation = match args.next() {
                None => None,
                Some(flag) if flag == "--generation" => Some(
                    required_utf8_arg(&mut args, "generation")?
                        .parse::<u64>()
                        .map_err(|error| format!("invalid generation: {error}"))?,
                ),
                Some(flag) => {
                    return Err(format!("unexpected argument: {}", flag.to_string_lossy()));
                }
            };
            if args.next().is_some() {
                return Err("unexpected trailing argument".into());
            }

            run_object_store_query(root, namespace, query_json, generation).await
        } else {
            let path = PathBuf::from(first);
            let query_json = required_utf8_arg(&mut args, "query JSON")?;
            if args.next().is_some() {
                return Err("expected exactly a DBF path and query JSON".into());
            }
            run_dbf_query(path, query_json).await
        }
    }

    fn usage() -> String {
        "usage: wasi-query-stream <dbf-path> <query-json> | --object-store <root> <namespace> <query-json> [--generation <generation>]".into()
    }

    fn required_arg(
        args: &mut impl Iterator<Item = OsString>,
        name: &str,
    ) -> Result<OsString, String> {
        args.next().ok_or_else(|| format!("missing {name}"))
    }

    fn required_utf8_arg(
        args: &mut impl Iterator<Item = OsString>,
        name: &str,
    ) -> Result<String, String> {
        required_arg(args, name)?
            .into_string()
            .map_err(|_| format!("{name} must be valid UTF-8"))
    }

    async fn run_dbf_query(path: PathBuf, query_json: String) -> Result<(), String> {
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let table = txbase::dbf::DbfTable::from_bytes(&bytes).map_err(|error| error.to_string())?;
        let request = query::parse(query_json.as_bytes()).map_err(|error| error.to_string())?;
        let stream = query::stream_query(&table, &request).map_err(|error| error.to_string())?;
        stream_to_stdout(stream).await
    }

    async fn run_object_store_query(
        root: PathBuf,
        namespace: String,
        query_json: String,
        generation: Option<u64>,
    ) -> Result<(), String> {
        let store = SyncObjectStoreAdapter::new(ReadOnlyFilesystemObjectStore::new(root));
        let table = AsyncObjectTable::new(store, namespace).map_err(|error| error.to_string())?;
        let request = query::parse(query_json.as_bytes()).map_err(|error| error.to_string())?;
        let stream = match generation {
            Some(generation) => table.query_stream_at(generation, request),
            None => table.query_stream(request),
        }
        .map_err(|error| error.to_string())?;
        stream_to_stdout(stream).await
    }

    async fn stream_to_stdout<S>(stream: S) -> Result<(), String>
    where
        S: AsyncQueryStream<Item = Result<serde_json::Value, QueryError>>,
    {
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
