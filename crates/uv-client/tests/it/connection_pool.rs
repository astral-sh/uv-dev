use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use futures::future::try_join_all;
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::{Barrier, oneshot};
use tokio::task::JoinSet;

use uv_client::BaseClientBuilder;
use uv_configuration::Concurrency;

#[tokio::test]
async fn reuses_connections_across_download_bursts() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let accepted = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(Concurrency::DEFAULT_DOWNLOADS));
    let (shutdown, mut stopping) = oneshot::channel();
    let server = tokio::spawn({
        let accepted = Arc::clone(&accepted);
        async move {
            let mut connections = JoinSet::new();
            loop {
                let (stream, _) = tokio::select! {
                    result = listener.accept() => result?,
                    _ = &mut stopping => break,
                };
                accepted.fetch_add(1, Ordering::SeqCst);
                let barrier = Arc::clone(&barrier);
                connections.spawn(async move {
                    http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(stream),
                            service_fn(move |_: Request<Incoming>| {
                                let barrier = Arc::clone(&barrier);
                                async move {
                                    // Hold the whole burst until every request has a connection.
                                    barrier.wait().await;
                                    Ok::<_, Infallible>(Response::new(Full::new(
                                        Bytes::from_static(b"ok"),
                                    )))
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

    let client = BaseClientBuilder::default().build()?;
    let url = format!("http://{address}/");
    for _ in 0..2 {
        tokio::time::timeout(
            Duration::from_secs(10),
            try_join_all((0..Concurrency::DEFAULT_DOWNLOADS).map(|_| async {
                let body = client.raw_client().get(&url).send().await?.bytes().await?;
                assert_eq!(body, "ok");
                Ok::<_, reqwest::Error>(())
            })),
        )
        .await??;
    }
    let count = accepted.load(Ordering::SeqCst);
    let _ = shutdown.send(());
    server.await??;
    assert_eq!(count, Concurrency::DEFAULT_DOWNLOADS);
    Ok(())
}
