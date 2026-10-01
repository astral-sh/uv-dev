use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use futures::stream;
use http::HeaderValue;
use http::header::RETRY_AFTER;
use http_body_util::StreamBody;
use hyper::body::{Bytes, Frame, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use uv_client::BaseClientBuilder;
use uv_redacted::DisplaySafeUrl;

#[tokio::test]
async fn reuses_connection_after_small_chunked_retry_response() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let accepted = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let (shutdown, mut stopping) = oneshot::channel();
    let server = tokio::spawn({
        let accepted = Arc::clone(&accepted);
        let requests = Arc::clone(&requests);
        async move {
            let mut connections = JoinSet::new();
            loop {
                let (stream, _) = tokio::select! {
                    result = listener.accept() => result?,
                    _ = &mut stopping => break,
                };
                accepted.fetch_add(1, Ordering::SeqCst);
                let requests = Arc::clone(&requests);
                connections.spawn(async move {
                    http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(stream),
                            service_fn(move |_request: Request<Incoming>| {
                                let first = requests.fetch_add(1, Ordering::SeqCst) == 0;
                                async move {
                                    // Delay the unknown-length body so the headers cannot arrive
                                    // together with the complete response.
                                    let body = StreamBody::new(stream::once(async move {
                                        tokio::time::sleep(Duration::from_millis(20)).await;
                                        Ok::<_, Infallible>(Frame::data(Bytes::from_static(
                                            if first { b"try again" } else { b"ok" },
                                        )))
                                    }));
                                    let mut response = Response::new(body);
                                    if first {
                                        *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
                                        response
                                            .headers_mut()
                                            .insert(RETRY_AFTER, HeaderValue::from_static("1"));
                                    }
                                    Ok::<_, Infallible>(response)
                                }
                            }),
                        )
                        .await
                });
            }
            connections.shutdown().await;
            Ok::<_, std::io::Error>(())
        }
    });

    let client = BaseClientBuilder::default().retries(1).build()?;
    let url = DisplaySafeUrl::parse(&format!("http://{address}/retry"))?;
    let response = tokio::time::timeout(Duration::from_secs(10), async {
        client.for_host(&url).get(url.as_str()).send().await
    })
    .await??;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await?, "ok");
    let connections = accepted.load(Ordering::SeqCst);
    let request_count = requests.load(Ordering::SeqCst);
    let _ = shutdown.send(());
    server.await??;
    assert_eq!(request_count, 2);
    assert_eq!(connections, 1);
    Ok(())
}
