use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use futures::stream;
use http::header::AUTHORIZATION;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinSet;

use uv_auth::Credentials;
use uv_client::BaseClientBuilder;
use uv_redacted::DisplaySafeUrl;

struct ChallengeBody {
    cancelled: Arc<AtomicBool>,
    notification: Arc<Notify>,
    chunk: Bytes,
}

impl Drop for ChallengeBody {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.notification.notify_waiters();
    }
}

#[tokio::test]
async fn cancels_streaming_authentication_error_before_retrying() -> Result<()> {
    for status in [
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::NOT_FOUND,
    ] {
        cancels_streaming_response(status).await?;
    }
    Ok(())
}

async fn cancels_streaming_response(status: StatusCode) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let notification = Arc::new(Notify::new());
    let cancelled_before_response = Arc::new(AtomicBool::new(false));
    let (shutdown, mut stopping) = oneshot::channel();
    let server = tokio::spawn({
        let cancelled = Arc::clone(&cancelled);
        let notification = Arc::clone(&notification);
        let cancelled_before_response = Arc::clone(&cancelled_before_response);
        async move {
            let mut connections = JoinSet::new();
            loop {
                let (stream, _) = tokio::select! {
                    result = listener.accept() => result?,
                    _ = &mut stopping => break,
                };
                let cancelled = Arc::clone(&cancelled);
                let notification = Arc::clone(&notification);
                let cancelled_before_response = Arc::clone(&cancelled_before_response);
                connections.spawn(async move {
                    http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(stream),
                            service_fn(move |request: Request<Incoming>| {
                                let cancelled = Arc::clone(&cancelled);
                                let notification = Arc::clone(&notification);
                                let cancelled_before_response =
                                    Arc::clone(&cancelled_before_response);
                                async move {
                                    let authenticated = request
                                        .headers()
                                        .get(AUTHORIZATION)
                                        .is_some_and(|value| value == "Basic dXNlcjpwYXNzd29yZA==");
                                    if authenticated {
                                        let notified = notification.notified();
                                        tokio::pin!(notified);
                                        notified.as_mut().enable();
                                        let observed = cancelled.load(Ordering::SeqCst)
                                            || tokio::time::timeout(
                                                Duration::from_secs(2),
                                                notified,
                                            )
                                            .await
                                            .is_ok();
                                        cancelled_before_response.store(observed, Ordering::SeqCst);
                                        return Ok::<_, Infallible>(Response::new(
                                            Full::new(Bytes::from_static(b"ok")).boxed_unsync(),
                                        ));
                                    }

                                    let body = stream::unfold(
                                        ChallengeBody {
                                            cancelled,
                                            notification,
                                            chunk: Bytes::from(vec![b'!'; 64 * 1024]),
                                        },
                                        |state| async move {
                                            tokio::time::sleep(Duration::from_millis(10)).await;
                                            Some((
                                                Ok::<_, Infallible>(Frame::data(
                                                    state.chunk.clone(),
                                                )),
                                                state,
                                            ))
                                        },
                                    );
                                    let mut response =
                                        Response::new(StreamBody::new(body).boxed_unsync());
                                    *response.status_mut() = status;
                                    Ok(response)
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
    let observed = cancelled_before_response.load(Ordering::SeqCst);
    let _ = shutdown.send(());
    server.await??;
    assert!(observed);
    Ok(())
}
