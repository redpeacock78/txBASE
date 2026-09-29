#[cfg(all(target_arch = "wasm32", target_os = "wasi", target_env = "p2"))]
#[path = "wasi_query_stream/output.rs"]
mod output;

#[cfg(all(target_arch = "wasm32", target_os = "wasi", target_env = "p2"))]
mod component {
    use std::cell::RefCell;
    use std::future::{Future, poll_fn};
    use std::pin::Pin;
    use std::rc::Rc;
    use std::task::{Context, Poll, Waker};

    use txbase::query::{AsyncQueryStream, QueryError};

    wasip3::cli::command::export!(WasiPendingQueryStream);

    struct WasiPendingQueryStream;

    impl wasip3::exports::cli::run::Guest for WasiPendingQueryStream {
        async fn run() -> Result<(), ()> {
            let wake_slot = Rc::new(RefCell::new(None));
            let stream = PendingOnceStream {
                state: 0,
                wake_slot: Rc::clone(&wake_slot),
            };
            let resume = poll_fn(move |_| match wake_slot.borrow_mut().take() {
                Some(waker) => {
                    waker.wake();
                    Poll::Ready(())
                }
                None => Poll::Pending,
            });
            let mut output = Box::pin(crate::output::stream_to_stdout(stream));
            let mut resume = Box::pin(resume);
            let mut wake_delivered = false;
            let result = poll_fn(|context| {
                if let Poll::Ready(result) = output.as_mut().poll(&mut *context) {
                    if result.is_ok() && !wake_delivered {
                        return Poll::Ready(Err(
                            "stream completed before its pending wake was delivered".into(),
                        ));
                    }
                    return Poll::Ready(result);
                }

                if !wake_delivered && resume.as_mut().poll(&mut *context).is_ready() {
                    wake_delivered = true;
                    return Poll::Pending;
                }
                Poll::Pending
            })
            .await;

            match result {
                Ok(()) => Ok(()),
                Err(error) => {
                    eprintln!("{error}");
                    Err(())
                }
            }
        }
    }

    struct PendingOnceStream {
        state: u8,
        wake_slot: Rc<RefCell<Option<Waker>>>,
    }

    impl AsyncQueryStream for PendingOnceStream {
        type Item = Result<serde_json::Value, QueryError>;

        fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let stream = self.get_mut();
            match stream.state {
                0 => {
                    stream.state = 1;
                    *stream.wake_slot.borrow_mut() = Some(context.waker().clone());
                    Poll::Pending
                }
                1 => {
                    stream.state = 2;
                    Poll::Ready(Some(Ok(serde_json::json!({"state": "resumed"}))))
                }
                _ => Poll::Ready(None),
            }
        }
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "wasi", target_env = "p2")))]
#[allow(dead_code)]
fn main() {}
