use std::convert::Infallible;
use std::future::ready;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::StatusCode;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio_stream::wrappers::ReceiverStream;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

pub(super) fn connection_reset(_request: &wiremock::Request) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionReset, "Connection reset by peer")
}

/// Answers with a retryable HTTP status 500.
pub(super) async fn http_error_server() -> (MockServer, String) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(StatusCode::INTERNAL_SERVER_ERROR))
        .mount(&server)
        .await;

    let mock_server_uri = server.uri();
    (server, mock_server_uri)
}

/// Answers with a retryable connection reset IO error.
pub(super) async fn io_error_server() -> (MockServer, String) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with_err(connection_reset)
        .mount(&server)
        .await;

    let mock_server_uri = server.uri();
    (server, mock_server_uri)
}

pub(super) type StreamingResponse = hyper::Response<BoxBody<Bytes, Infallible>>;

pub(super) struct StreamingServerGuard {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for StreamingServerGuard {
    fn drop(&mut self) {
        drop(self.shutdown.take());
        if let Err(panic) = self.thread.take().unwrap().join()
            && !std::thread::panicking()
        {
            std::panic::resume_unwind(panic);
        }
    }
}

/// Emit some bytes, then wait before ending the response body.
pub(super) fn delayed_body(bytes: Bytes, delay: Duration) -> BoxBody<Bytes, Infallible> {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        let _ = tx.send(Ok(Frame::data(bytes))).await;
        tokio::time::sleep(delay).await;
    });
    StreamBody::new(ReceiverStream::new(rx)).boxed()
}

pub(super) fn time_out_response(
    _request: hyper::Request<hyper::body::Incoming>,
) -> Result<StreamingResponse, http::Error> {
    hyper::Response::builder()
        .header("Content-Type", "text/html")
        .body(delayed_body(Bytes::new(), Duration::from_mins(1)))
}

/// Run a streaming HTTP server on its own runtime so test subprocesses cannot starve it.
/// Dropping the guard shuts down and joins the server, propagating handler panics to the test.
pub(super) fn streaming_server<F>(handler: F) -> (String, StreamingServerGuard)
where
    F: Fn(hyper::Request<hyper::body::Incoming>) -> Result<StreamingResponse, http::Error>
        + Send
        + Sync
        + 'static,
{
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = format!("http://{}", listener.local_addr().unwrap());
    let handler = Arc::new(handler);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, _) = accepted.expect("streaming server failed to accept connection");
                        let handler = handler.clone();
                        connections.spawn(async move {
                            let _ = hyper_util::server::conn::auto::Builder::new(
                                hyper_util::rt::TokioExecutor::new(),
                            )
                            .serve_connection(TokioIo::new(stream), service_fn(move |request| {
                                ready(handler(request))
                            }))
                            .await;
                        });
                    }
                    Some(result) = connections.join_next() => result.unwrap(),
                    _ = &mut shutdown_rx => break,
                }
            }
            connections.abort_all();
            while let Some(result) = connections.join_next().await {
                if let Err(error) = result
                    && error.is_panic()
                {
                    std::panic::resume_unwind(error.into_panic());
                }
            }
        });
    });
    (
        server,
        StreamingServerGuard {
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        },
    )
}
