use std::future::poll_fn;

use txbase::query::{AsyncQueryStream, QueryError};

pub(crate) async fn stream_to_stdout<S>(stream: S) -> Result<(), String>
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
