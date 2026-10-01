use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, SystemTimeError};
use std::{io, iter};

use anyhow::anyhow;
use http::header::RETRY_AFTER;
use http::status::StatusCode;
use http::{Extensions, HeaderMap};
use itertools::Itertools;
use reqwest::{Request, Response};
use reqwest_middleware::{Error as MiddlewareError, Middleware, Next};
use reqwest_retry::policies::ExponentialBackoff;
use reqwest_retry::{
    RetryCount, RetryDecision, RetryError, RetryPolicy, Retryable, RetryableStrategy,
    default_on_request_error, default_on_request_success,
};
use rustls::{AlertDescription, Error as RustlsError};
use tracing::{debug, trace, warn};
use url::Url;

use uv_redacted::DisplaySafeUrl;

use crate::{RequestBuilder, WrappedReqwestError};

/// An extension over [`DefaultRetryableStrategy`] that logs transient request failures and
/// adds additional retry cases.
struct UvRetryableStrategy;

impl RetryableStrategy for UvRetryableStrategy {
    fn handle(&self, res: &Result<Response, reqwest_middleware::Error>) -> Option<Retryable> {
        let retryable = match res {
            Ok(success) => default_on_request_success(success),
            Err(err) => retryable_on_request_failure(err),
        };

        // Log on transient errors
        if retryable == Some(Retryable::Transient) {
            match res {
                Ok(response) => {
                    debug!(
                        "Transient request failure for: {}",
                        DisplaySafeUrl::ref_cast(response.url())
                    );
                }
                Err(err) => {
                    let url = request_error_url(err).map(DisplaySafeUrl::ref_cast);
                    let redact = |message: &str| {
                        url.map_or_else(|| message.to_owned(), |url| url.redact_in(message))
                    };
                    let context = iter::successors(err.source(), |&err| err.source())
                        .map(|err| format!("  Caused by: {}", redact(&err.to_string())))
                        .join("\n");
                    let error = redact(&err.to_string());
                    debug!(
                        "Transient request failure for {}, retrying: {error}\n{context}",
                        url.map_or_else(|| "unknown URL".to_owned(), ToString::to_string)
                    );
                }
            }
        }
        retryable
    }
}

/// Retry transient requests using server advice within the configured retry limits.
pub(crate) struct UvRetryMiddleware {
    policy: ExponentialBackoff,
}

impl UvRetryMiddleware {
    pub(crate) fn new(policy: ExponentialBackoff) -> Self {
        Self { policy }
    }
}

#[async_trait::async_trait]
impl Middleware for UvRetryMiddleware {
    async fn handle(
        &self,
        request: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        // Keep server advice local to this request, including all of its retry attempts.
        let retry_after = Arc::new(Mutex::new(None));
        let policy = RetryAfterPolicy {
            policy: self.policy,
            retry_after: retry_after.clone(),
        };
        let strategy = RetryAfterStrategy {
            max_delay: self.policy.max_retry_interval,
            retry_after,
        };
        let start = SystemTime::now();
        let mut past_retries = 0;
        loop {
            let duplicate = request.try_clone().ok_or_else(|| {
                MiddlewareError::Middleware(anyhow!(
                    "Request object is not cloneable. Are you passing a streaming body?"
                ))
            })?;
            let result = next.clone().run(duplicate, extensions).await;
            if strategy.handle(&result) == Some(Retryable::Transient)
                && let RetryDecision::Retry { execute_after } =
                    policy.should_retry(start, past_retries)
            {
                // An unread response can retain HTTP/2 flow-control capacity or continue using
                // bandwidth. Release it before the backoff so other requests can make progress.
                drop(result);
                let delay = execute_after
                    .duration_since(SystemTime::now())
                    .unwrap_or_default();
                warn!(
                    "Retry attempt #{}. Sleeping {:?} before the next attempt",
                    past_retries, delay
                );
                tokio::time::sleep(delay).await;
                past_retries += 1;
                continue;
            }

            return match result {
                Ok(mut response) => {
                    if past_retries > 0 {
                        response
                            .extensions_mut()
                            .insert(RetryCount::new(past_retries));
                    }
                    Ok(response)
                }
                Err(error) => {
                    let error = if past_retries == 0 {
                        RetryError::Error(error)
                    } else {
                        RetryError::WithRetries {
                            retries: past_retries,
                            err: error,
                        }
                    };
                    Err(MiddlewareError::Middleware(error.into()))
                }
            };
        }
    }
}

struct RetryAfterPolicy {
    policy: ExponentialBackoff,
    retry_after: Arc<Mutex<Option<SystemTime>>>,
}

impl RetryPolicy for RetryAfterPolicy {
    fn should_retry(&self, start: SystemTime, past_retries: u32) -> RetryDecision {
        match self.policy.should_retry(start, past_retries) {
            decision @ RetryDecision::DoNotRetry => decision,
            decision @ RetryDecision::Retry { .. } => self
                .retry_after
                .lock()
                .expect("Retry-After state poisoned")
                .take()
                .map_or(decision, |execute_after| RetryDecision::Retry {
                    execute_after,
                }),
        }
    }
}

struct RetryAfterStrategy {
    max_delay: Duration,
    retry_after: Arc<Mutex<Option<SystemTime>>>,
}

impl RetryableStrategy for RetryAfterStrategy {
    fn handle(&self, response: &reqwest_middleware::Result<Response>) -> Option<Retryable> {
        let retryable = UvRetryableStrategy.handle(response);
        let execute_after = if retryable == Some(Retryable::Transient) {
            response.as_ref().ok().and_then(|response| {
                let now = SystemTime::now();
                retry_after(response.headers(), now, self.max_delay)
                    .and_then(|delay| now.checked_add(delay))
            })
        } else {
            None
        };
        *self.retry_after.lock().expect("Retry-After state poisoned") = execute_after;
        retryable
    }
}

/// Parse `Retry-After` without allowing an origin to exceed the client's maximum retry delay.
fn retry_after(headers: &HeaderMap, now: SystemTime, max_delay: Duration) -> Option<Duration> {
    let mut values = headers.get_all(RETRY_AFTER).iter();
    let value = values.next()?.to_str().ok()?.trim();
    if values.next().is_some() {
        return None;
    }
    let duration = if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        Duration::from_secs(value.parse::<u64>().unwrap_or(u64::MAX))
    } else {
        httpdate::parse_http_date(value)
            .ok()?
            .duration_since(now)
            .unwrap_or_default()
    };
    Some(duration.min(max_delay))
}

/// Per-request retry state and policy.
pub struct RetryState {
    retry_policy: ExponentialBackoff,
    start_time: SystemTime,
    total_retries: u32,
    url: DisplaySafeUrl,
}

impl RetryState {
    /// Initialize the [`RetryState`] and record the start time for the retry policy.
    pub fn start(retry_policy: ExponentialBackoff, url: impl Into<DisplaySafeUrl>) -> Self {
        Self {
            retry_policy,
            start_time: SystemTime::now(),
            total_retries: 0,
            url: url.into(),
        }
    }

    /// The number of retries across all requests.
    ///
    /// Includes retries reported by the HTTP middleware.
    pub(crate) fn total_retries(&self) -> u32 {
        self.total_retries
    }

    /// The total duration from the first request to the (failure) of the last request.
    pub(crate) fn duration(&self) -> Result<Duration, SystemTimeError> {
        self.start_time.elapsed()
    }

    /// Send a request and count any retries performed by the middleware.
    pub async fn send(
        &mut self,
        request: RequestBuilder<'_>,
    ) -> reqwest_middleware::Result<Response> {
        let result = request.send().await;
        self.record_request_retries(result.as_ref());
        result
    }

    /// Count middleware retries before invoking the callback with the updated [`RetryState`].
    pub(crate) async fn handle_response<Payload, CallbackError, Callback>(
        &mut self,
        response: Response,
        callback: Callback,
    ) -> Result<Payload, CallbackError>
    where
        Callback: AsyncFnOnce(Response, &mut Self) -> Result<Payload, CallbackError>,
    {
        self.record_request_retries(Ok(&response));
        callback(response, self).await
    }

    /// Account for retries performed by the middleware before handling a response or error.
    ///
    /// Call once per request, including successful requests whose bodies may later fail.
    fn record_request_retries(&mut self, result: Result<&Response, &reqwest_middleware::Error>) {
        let retries = match result {
            Ok(response) => response
                .extensions()
                .get::<reqwest_retry::RetryCount>()
                .map_or(0, |retries| retries.value()),
            Err(reqwest_middleware::Error::Middleware(err)) => {
                match err.downcast_ref::<reqwest_retry::RetryError>() {
                    Some(reqwest_retry::RetryError::WithRetries { retries, .. }) => *retries,
                    Some(reqwest_retry::RetryError::Error(_)) | None => 0,
                }
            }
            Err(reqwest_middleware::Error::Reqwest(_)) => 0,
        };
        self.total_retries += retries;
    }

    /// Determines whether request should be retried.
    ///
    /// Takes the number of retries from nested layers associated with the specific `err` type as
    /// `error_retries`.
    ///
    /// Returns the backoff duration if the request should be retried.
    #[must_use]
    pub fn should_retry(
        &mut self,
        err: &(dyn Error + 'static),
        error_retries: u32,
    ) -> Option<Duration> {
        // If the middleware performed any retries, consider them in our budget.
        self.total_retries += error_retries;
        match retryable_on_request_failure(err) {
            Some(Retryable::Transient) => {
                // Capture `now` before calling the policy so that `execute_after`
                // (computed from a `SystemTime::now()` inside the library) is always
                // >= `now`, making `duration_since` reliable.
                let now = SystemTime::now();
                let retry_decision = self
                    .retry_policy
                    .should_retry(self.start_time, self.total_retries);
                if let reqwest_retry::RetryDecision::Retry { execute_after } = retry_decision {
                    let duration = execute_after
                        .duration_since(now)
                        .unwrap_or_else(|_| Duration::default());

                    self.total_retries += 1;
                    return Some(duration);
                }

                None
            }
            Some(Retryable::Fatal) | None => None,
        }
    }

    /// Wait before retrying the request.
    pub async fn sleep_backoff(&self, duration: Duration) {
        debug!(
            "Transient failure while handling response from {}; retrying after {:.1}s...",
            self.url,
            duration.as_secs_f32(),
        );
        // TODO(konsti): Should we show a spinner plus a message in the CLI while
        // waiting?
        tokio::time::sleep(duration).await;
    }
}

/// Whether the error looks like a network error that should be retried.
///
/// This is an extension over [`reqwest_middleware::default_on_request_failure`], which is missing
/// a number of cases:
/// * Inside the reqwest or reqwest-middleware error is an `io::Error` such as a broken pipe
/// * When streaming a response, a reqwest error may be hidden several layers behind errors
///   of different crates processing the stream, including `io::Error` layers
/// * Any `h2` error
pub fn retryable_on_request_failure(err: &(dyn Error + 'static)) -> Option<Retryable> {
    // First, try to show a nice trace log
    if let Some((Some(status), Some(url))) = find_source::<WrappedReqwestError>(&err)
        .map(|request_err| (request_err.status(), request_err.url()))
    {
        trace!(
            "Considering retry of response HTTP {status} for {url}",
            url = DisplaySafeUrl::from_url(url.clone())
        );
    } else if let Some(url) = request_error_url(err) {
        trace!(
            "Considering retry of error: {}",
            DisplaySafeUrl::ref_cast(url).redact_in(&format!("{err:?}"))
        );
    } else {
        trace!("Considering retry of error: {err:?}");
    }

    let mut has_known_error = false;
    // IO Errors or reqwest errors may be nested through custom IO errors or stream processing
    // crates
    let mut current_source = Some(err);
    while let Some(source) = current_source {
        // Handle different kinds of reqwest error nesting not accessible by downcast.
        let reqwest_err = if let Some(reqwest_err) = source.downcast_ref::<reqwest::Error>() {
            Some(reqwest_err)
        } else if let Some(reqwest_err) = source
            .downcast_ref::<WrappedReqwestError>()
            .and_then(|err| err.inner())
        {
            Some(reqwest_err)
        } else if let Some(reqwest_middleware::Error::Reqwest(reqwest_err)) =
            source.downcast_ref::<reqwest_middleware::Error>()
        {
            Some(reqwest_err)
        } else {
            None
        };

        if let Some(reqwest_err) = reqwest_err {
            has_known_error = true;
            if is_tls_certificate_error(reqwest_err) {
                trace!("Fatal nested reqwest TLS certificate error");
                return Some(Retryable::Fatal);
            }
            // Ignore the default retry strategy returning fatal.
            if default_on_request_error(reqwest_err) == Some(Retryable::Transient) {
                trace!("Transient nested reqwest error");
                return Some(Retryable::Transient);
            }
            if is_retryable_status_error(reqwest_err) {
                trace!("Transient nested reqwest status code error");
                return Some(Retryable::Transient);
            }

            trace!("Fatal nested reqwest error");
        } else if source.downcast_ref::<h2::Error>().is_some() {
            // All h2 errors look like errors that should be retried
            // https://github.com/astral-sh/uv/issues/15916
            trace!("Transient nested h2 error");
            return Some(Retryable::Transient);
        } else if let Some(io_err) = source.downcast_ref::<io::Error>() {
            has_known_error = true;
            let retryable_io_err_kinds = [
                // https://github.com/astral-sh/uv/issues/12054
                io::ErrorKind::BrokenPipe,
                // From reqwest-middleware
                io::ErrorKind::ConnectionAborted,
                // https://github.com/astral-sh/uv/issues/3514
                io::ErrorKind::ConnectionReset,
                // https://github.com/astral-sh/uv/issues/14699
                io::ErrorKind::InvalidData,
                // https://github.com/astral-sh/uv/issues/17697#issuecomment-3817060484
                io::ErrorKind::TimedOut,
                // https://github.com/astral-sh/uv/issues/9246
                io::ErrorKind::UnexpectedEof,
            ];
            if retryable_io_err_kinds.contains(&io_err.kind()) {
                trace!("Transient IO error: `{}`", io_err.kind());
                return Some(Retryable::Transient);
            }

            trace!(
                "Fatal IO error `{}`, not a transient IO error kind",
                io_err.kind()
            );
        }

        current_source = source.source();
    }

    if !has_known_error {
        trace!("Cannot retry error: neither an IO error nor a reqwest error");
    }

    None
}

/// An error type that supports URL-fallback and exponential-backoff retry logic.
///
/// Used by [`fetch_with_url_fallback`] to drive the retry loop without knowing the concrete error
/// type.
pub trait RetriableError: std::error::Error + Sized + 'static {
    /// Returns `true` if an alternative URL should be tried immediately (without backoff).
    fn should_try_next_url(&self) -> bool;

    /// Returns the number of inner retries already recorded in this error.
    fn retries(&self) -> u32;

    /// Wrap the error to indicate that the operation was retried `retries` times before failing.
    #[must_use]
    fn into_retried(self, retries: u32, duration: Duration) -> Self;
}

/// Whether the error is a status code error that is retryable.
///
/// Port of `reqwest_retry::default_on_request_success`.
fn is_retryable_status_error(reqwest_err: &reqwest::Error) -> bool {
    let Some(status) = reqwest_err.status() else {
        return false;
    };
    status.is_server_error()
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
}

fn is_tls_certificate_error(reqwest_err: &reqwest::Error) -> bool {
    let Some(rustls_error) = find_source::<RustlsError>(reqwest_err) else {
        return false;
    };

    // TODO(konsti): https://github.com/seanmonstar/reqwest/issues/2819#issuecomment-5032072023
    match rustls_error {
        RustlsError::InvalidCertificate(_) | RustlsError::NoCertificatesPresented => true,
        RustlsError::AlertReceived(alert) => matches!(
            alert,
            AlertDescription::AccessDenied
                | AlertDescription::BadCertificate
                | AlertDescription::BadCertificateHashValue
                | AlertDescription::BadCertificateStatusResponse
                | AlertDescription::CertificateExpired
                | AlertDescription::CertificateRequired
                | AlertDescription::CertificateRevoked
                | AlertDescription::CertificateUnknown
                | AlertDescription::CertificateUnobtainable
                | AlertDescription::DecryptError
                | AlertDescription::NoCertificate
                | AlertDescription::UnknownCA
                | AlertDescription::UnsupportedCertificate
        ),
        _ => false,
    }
}

/// Finds the request URL for diagnostics, including transparent middleware and retry wrappers.
fn request_error_url<'a>(err: &'a (dyn Error + 'static)) -> Option<&'a Url> {
    iter::successors(Some(err), |&err| {
        if let Some(io_error) = err.downcast_ref::<io::Error>()
            && let Some(inner) = io_error.get_ref()
        {
            Some(inner as &(dyn Error + 'static))
        } else {
            err.source()
        }
    })
    .find_map(|err| {
        err.downcast_ref::<reqwest::Error>()
            .and_then(reqwest::Error::url)
            .or_else(|| {
                err.downcast_ref::<reqwest_middleware::Error>()
                    .and_then(reqwest_middleware::Error::url)
            })
            .or_else(|| {
                err.downcast_ref::<WrappedReqwestError>()
                    .and_then(|err| err.url())
            })
    })
}

/// Find the first source error of a specific type, including errors wrapped by [`io::Error`].
///
/// Inspired by <https://github.com/seanmonstar/reqwest/issues/1602#issuecomment-1220996681>
/// See <https://github.com/hyperium/h2/issues/862>
fn find_source<E: Error + 'static>(orig: &dyn Error) -> Option<&E> {
    let mut cause = orig.source();
    while let Some(err) = cause {
        if let Some(concrete_err) = err.downcast_ref() {
            return Some(concrete_err);
        }
        if let Some(io_err) = err.downcast_ref::<io::Error>()
            && let Some(inner_err) = io_err.get_ref()
        {
            if let Some(concrete_err) = inner_err.downcast_ref() {
                return Some(concrete_err);
            }
            cause = Some(inner_err);
            continue;
        }
        cause = err.source();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::future::pending;

    use anyhow::{Result, anyhow};
    use futures::stream;
    use insta::assert_debug_snapshot;
    use reqwest::{Body, Client};
    use reqwest_middleware::ClientWithMiddleware;
    use tokio::sync::oneshot;
    use tracing_test::traced_test;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::retryable_on_request_failure;

    struct ResponseDropGuard(Option<oneshot::Sender<()>>);

    impl Drop for ResponseDropGuard {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    struct ResponseQueue(Mutex<VecDeque<reqwest_middleware::Result<Response>>>);

    #[derive(Debug, thiserror::Error)]
    #[error("Injected transport failure")]
    struct InjectedTransportError(#[source] io::Error);

    #[async_trait::async_trait]
    impl Middleware for ResponseQueue {
        async fn handle(
            &self,
            _request: Request,
            _extensions: &mut Extensions,
            _next: Next<'_>,
        ) -> reqwest_middleware::Result<Response> {
            self.0
                .lock()
                .expect("Response queue poisoned")
                .pop_front()
                .ok_or_else(|| {
                    reqwest_middleware::Error::Middleware(anyhow!("No response queued"))
                })?
        }
    }

    fn unfinished_response(status: StatusCode) -> (Response, oneshot::Receiver<()>) {
        let (sender, receiver) = oneshot::channel();
        let guard = ResponseDropGuard(Some(sender));
        let body = Body::wrap_stream(stream::once(async move {
            let _guard = guard;
            pending::<std::result::Result<Vec<u8>, io::Error>>().await
        }));
        let mut response = http::Response::new(body);
        *response.status_mut() = status;
        (response.into(), receiver)
    }

    #[tokio::test]
    async fn retry_releases_response_before_backoff() -> Result<()> {
        let (response, released) = unfinished_response(StatusCode::SERVICE_UNAVAILABLE);
        let client = reqwest_middleware::ClientBuilder::new(Client::new())
            .with(UvRetryMiddleware::new(
                ExponentialBackoff::builder()
                    .jitter(reqwest_retry::Jitter::None)
                    .retry_bounds(Duration::from_secs(60), Duration::from_secs(60))
                    .build_with_max_retries(1),
            ))
            .with(ResponseQueue(Mutex::new(VecDeque::from([Ok(response)]))))
            .build();
        let request =
            tokio::spawn(async move { client.get("http://127.0.0.1/retry").send().await });
        let released = tokio::time::timeout(Duration::from_secs(2), released).await;
        let waiting = !request.is_finished();
        request.abort();
        let _ = request.await;
        assert!(waiting, "The retry did not honor its backoff");
        released??;
        Ok(())
    }

    #[tokio::test]
    async fn retry_retains_the_terminal_response_body() -> Result<()> {
        let (first, first_released) = unfinished_response(StatusCode::SERVICE_UNAVAILABLE);
        let (last, mut last_released) = unfinished_response(StatusCode::SERVICE_UNAVAILABLE);
        let client = reqwest_middleware::ClientBuilder::new(Client::new())
            .with(UvRetryMiddleware::new(
                ExponentialBackoff::builder()
                    .retry_bounds(Duration::ZERO, Duration::ZERO)
                    .build_with_max_retries(1),
            ))
            .with(ResponseQueue(Mutex::new(VecDeque::from([
                Ok(first),
                Ok(last),
            ]))))
            .build();
        let response = client.get("http://127.0.0.1/retry").send().await?;
        first_released.await?;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response
                .extensions()
                .get::<reqwest_retry::RetryCount>()
                .map(|count| count.value()),
            Some(1)
        );
        assert_eq!(
            last_released.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        );
        drop(response);
        last_released.await?;
        Ok(())
    }

    #[tokio::test]
    async fn retry_preserves_transport_error_counts() -> Result<()> {
        for retries in [0, 2] {
            let responses = Arc::new(ResponseQueue(Mutex::new(
                (0..=retries)
                    .map(|_| {
                        Err(MiddlewareError::middleware(InjectedTransportError(
                            io::Error::from(io::ErrorKind::ConnectionReset),
                        )))
                    })
                    .collect(),
            )));
            let client = reqwest_middleware::ClientBuilder::new(Client::new())
                .with(UvRetryMiddleware::new(
                    ExponentialBackoff::builder()
                        .retry_bounds(Duration::ZERO, Duration::ZERO)
                        .build_with_max_retries(retries),
                ))
                .with_arc(responses.clone())
                .build();
            let error = client
                .get("http://127.0.0.1/retry")
                .send()
                .await
                .expect_err("The transport error should exhaust its retry budget");
            let error = match error {
                MiddlewareError::Middleware(error) => error.downcast::<RetryError>()?,
                MiddlewareError::Reqwest(error) => return Err(error.into()),
            };
            let (observed, error) = match error {
                RetryError::Error(error) => (0, error),
                RetryError::WithRetries { retries, err } => (retries, err),
            };
            assert_eq!(observed, retries);
            assert_eq!(
                find_source::<io::Error>(&error).map(io::Error::kind),
                Some(io::ErrorKind::ConnectionReset)
            );
            assert!(
                responses
                    .0
                    .lock()
                    .expect("Response queue poisoned")
                    .is_empty()
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn retry_rejects_streaming_request_bodies_before_sending() -> Result<()> {
        let responses = Arc::new(ResponseQueue(Mutex::new(VecDeque::new())));
        let client = reqwest_middleware::ClientBuilder::new(Client::new())
            .with(UvRetryMiddleware::new(
                ExponentialBackoff::builder().build_with_max_retries(1),
            ))
            .with_arc(responses.clone())
            .build();
        let error = client
            .post("http://127.0.0.1/retry")
            .body(Body::wrap_stream(stream::pending::<
                std::result::Result<Vec<u8>, io::Error>,
            >()))
            .send()
            .await
            .expect_err("A streaming request cannot be retried");
        assert_eq!(
            error.to_string(),
            "Request object is not cloneable. Are you passing a streaming body?"
        );
        assert!(
            responses
                .0
                .lock()
                .expect("Response queue poisoned")
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn retry_after_values() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let maximum = Duration::from_secs(30);
        for (value, expected) in [
            ("0".to_owned(), Some(Duration::ZERO)),
            ("7".to_owned(), Some(Duration::from_secs(7))),
            (u64::MAX.to_string(), Some(maximum)),
            ("18446744073709551616".to_owned(), Some(maximum)),
            (
                httpdate::fmt_http_date(now + Duration::from_secs(10)),
                Some(Duration::from_secs(10)),
            ),
            (
                httpdate::fmt_http_date(now - Duration::from_secs(10)),
                Some(Duration::ZERO),
            ),
            ("+1".to_owned(), None),
            ("-1".to_owned(), None),
            ("1.5".to_owned(), None),
            ("invalid".to_owned(), None),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(RETRY_AFTER, value.parse().unwrap());
            assert_eq!(retry_after(&headers, now, maximum), expected, "{value}");
            if expected.is_some() {
                assert_eq!(
                    retry_after(&headers, now, Duration::ZERO),
                    Some(Duration::ZERO)
                );
            }
        }
        let mut headers = HeaderMap::new();
        assert_eq!(retry_after(&headers, now, maximum), None);
        headers.append(RETRY_AFTER, "1".parse().unwrap());
        headers.append(RETRY_AFTER, "2".parse().unwrap());
        assert_eq!(retry_after(&headers, now, maximum), None);
    }

    #[test]
    fn retry_after_keeps_the_retry_budget_and_clears_stale_advice() {
        let now = SystemTime::now();
        let retry_after = Arc::new(Mutex::new(Some(now)));
        let policy = RetryAfterPolicy {
            policy: ExponentialBackoff::builder()
                .jitter(reqwest_retry::Jitter::None)
                .retry_bounds(Duration::from_secs(2), Duration::from_secs(30))
                .build_with_max_retries(1),
            retry_after: retry_after.clone(),
        };
        assert!(matches!(
            policy.should_retry(now, 1),
            RetryDecision::DoNotRetry
        ));
        assert!(
            matches!(policy.should_retry(now, 0), RetryDecision::Retry { execute_after } if execute_after == now)
        );
        *retry_after.lock().unwrap() = Some(now);
        let strategy = RetryAfterStrategy {
            max_delay: Duration::from_secs(30),
            retry_after: retry_after.clone(),
        };
        let response = http::Response::builder()
            .status(503)
            .body("")
            .unwrap()
            .into();
        assert!(matches!(
            strategy.handle(&Ok(response)),
            Some(Retryable::Transient)
        ));
        assert!(retry_after.lock().unwrap().is_none());
        assert!(
            matches!(policy.should_retry(now, 0), RetryDecision::Retry { execute_after } if execute_after.duration_since(now).unwrap() >= Duration::from_secs(2))
        );
    }

    #[tokio::test]
    async fn retry_after_is_request_scoped() -> Result<()> {
        let server = MockServer::start().await;
        for (name, header) in [("immediate", Some("0")), ("regular", None)] {
            let mut response = ResponseTemplate::new(503);
            if let Some(header) = header {
                response = response.insert_header("Retry-After", header);
            }
            Mock::given(path(format!("/{name}")))
                .respond_with(response)
                .up_to_n_times(1)
                .with_priority(1)
                .mount(&server)
                .await;
            Mock::given(path(format!("/{name}")))
                .respond_with(ResponseTemplate::new(200))
                .with_priority(2)
                .mount(&server)
                .await;
        }
        let client = reqwest_middleware::ClientBuilder::new(Client::new())
            .with(UvRetryMiddleware::new(
                ExponentialBackoff::builder()
                    .jitter(reqwest_retry::Jitter::None)
                    .retry_bounds(Duration::from_secs(60), Duration::from_secs(60))
                    .build_with_max_retries(1),
            ))
            .build();
        let regular = {
            let client = client.clone();
            let url = format!("{}/regular", server.uri());
            tokio::spawn(async move { client.get(url).send().await })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .any(|request| request.url.path() == "/regular")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            client.get(format!("{}/immediate", server.uri())).send(),
        )
        .await??;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .extensions()
                .get::<reqwest_retry::RetryCount>()
                .map(|count| count.value()),
            Some(1)
        );
        assert!(
            !regular.is_finished(),
            "Retry-After leaked to another request"
        );
        regular.abort();
        let _ = regular.await;
        Ok(())
    }

    #[tokio::test]
    #[traced_test]
    async fn retry_logs_redact_signed_urls() -> Result<()> {
        let server = MockServer::start().await;
        Mock::given(path("/wheel"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let response = Client::new()
            .get(format!(
                "{}/wheel?sig=azure-secret&X-Amz-Signature=aws-secret",
                server.uri()
            ))
            .send()
            .await?;
        assert!(matches!(
            UvRetryableStrategy.handle(&Ok(response)),
            Some(Retryable::Transient)
        ));

        let response = Client::new()
            .get(format!(
                "{}/wheel?sig=azure-secret&X-Amz-Signature=aws-secret",
                server.uri()
            ))
            .send()
            .await?;
        let err = response
            .error_for_status()
            .expect_err("expected a 503 response");
        assert!(matches!(
            UvRetryableStrategy.handle(&Err(err.into())),
            Some(Retryable::Transient)
        ));

        let response = Client::new()
            .get(format!(
                "{}/wheel?sig=azure-secret&X-Amz-Signature=aws-secret",
                server.uri()
            ))
            .send()
            .await?;
        let err = response
            .error_for_status()
            .expect_err("expected a 503 response");
        let err = reqwest_middleware::Error::middleware(reqwest_retry::RetryError::WithRetries {
            retries: 1,
            err: err.into(),
        });
        assert!(matches!(
            UvRetryableStrategy.handle(&Err(err)),
            Some(Retryable::Transient)
        ));

        logs_assert(|lines| {
            let logs = lines.join("\n");
            assert!(logs.contains("Transient request failure"));
            assert!(logs.contains("Considering retry"));
            assert!(logs.contains("sig=****&X-Amz-Signature=****"));
            assert!(!logs.contains("azure-secret"));
            assert!(!logs.contains("aws-secret"));
            Ok(())
        });
        Ok(())
    }

    /// Enumerate which status codes we are retrying.
    #[tokio::test]
    async fn retried_status_codes() -> Result<()> {
        let server = MockServer::start().await;
        let client = Client::default();
        let middleware_client = ClientWithMiddleware::default();
        let mut retried = Vec::new();
        for status in 100..599 {
            // Test all standard status codes and an example for a non-RFC code used in the wild.
            if StatusCode::from_u16(status)?.canonical_reason().is_none() && status != 420 {
                continue;
            }

            Mock::given(path(format!("/{status}")))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;

            let response = middleware_client
                .get(format!("{}/{}", server.uri(), status))
                .send()
                .await;

            let middleware_retry =
                UvRetryableStrategy.handle(&response) == Some(Retryable::Transient);

            let response = client
                .get(format!("{}/{}", server.uri(), status))
                .send()
                .await?;

            let uv_retry = match response.error_for_status() {
                Ok(_) => false,
                Err(err) => retryable_on_request_failure(&err) == Some(Retryable::Transient),
            };

            // Ensure we're retrying the same status code as the reqwest_retry crate. We may choose
            // to deviate from this later.
            assert_eq!(middleware_retry, uv_retry);
            if uv_retry {
                retried.push(status);
            }
        }

        assert_debug_snapshot!(retried, @"
        [
            100,
            102,
            103,
            408,
            429,
            500,
            501,
            502,
            503,
            504,
            505,
            506,
            507,
            508,
            510,
            511,
        ]
        ");

        Ok(())
    }
}
