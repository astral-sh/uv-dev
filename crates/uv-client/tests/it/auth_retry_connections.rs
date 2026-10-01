use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use http::header::AUTHORIZATION;
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use uv_auth::Credentials;
use uv_client::BaseClientBuilder;
use uv_redacted::DisplaySafeUrl;

#[tokio::test]
async fn reuses_connection_after_authentication_challenge() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let accepted = Arc::new(AtomicUsize::new(0));
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
                connections.spawn(async move {
                    http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(stream),
                            service_fn(|request: Request<Incoming>| async move {
                                let authenticated = request
                                    .headers()
                                    .get(AUTHORIZATION)
                                    .is_some_and(|value| value == "Basic dXNlcjpwYXNzd29yZA==");
                                let mut response = Response::new(Full::new(Bytes::from_static(
                                    if authenticated {
                                        b"ok"
                                    } else {
                                        b"authentication required"
                                    },
                                )));
                                if !authenticated {
                                    *response.status_mut() = StatusCode::UNAUTHORIZED;
                                }
                                Ok::<_, Infallible>(response)
                            }),
                        )
                        .await
                });
            }
            connections.shutdown().await;
            Ok::<_, std::io::Error>(())
        }
    });

    let builder = BaseClientBuilder::default();
    builder.store_credentials(
        &DisplaySafeUrl::parse(&format!("http://{address}/other/"))?,
        Credentials::basic(Some("user".to_owned()), Some("password".to_owned())),
    );
    let client = builder.build()?;
    let url = DisplaySafeUrl::parse(&format!("http://{address}/protected/"))?;
    let response = tokio::time::timeout(Duration::from_secs(10), async {
        client.for_host(&url).get(url.as_str()).send().await
    })
    .await??;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await?, "ok");
    let count = accepted.load(Ordering::SeqCst);
    let _ = shutdown.send(());
    server.await??;
    assert_eq!(count, 1);
    Ok(())
}
