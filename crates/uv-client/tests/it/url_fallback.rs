use std::assert_matches;
use std::error::Error as _;
use std::io;
use std::time::{Duration, SystemTimeError};

use anyhow::Result;
use reqwest_retry::policies::ExponentialBackoff;
use thiserror::Error;

use uv_client::{RetriableError, fetch_with_url_fallback};
use uv_redacted::DisplaySafeUrl;

#[derive(Debug, Error)]
enum FetchError {
    #[error("{message}")]
    Attempt {
        message: &'static str,
        fallback: bool,
        #[source]
        source: io::Error,
    },
    #[error("request failed after {retries} retries")]
    Retried {
        retries: u32,
        #[source]
        source: Box<Self>,
    },
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
}

impl FetchError {
    fn attempt(message: &'static str, fallback: bool, kind: io::ErrorKind) -> Self {
        Self::Attempt {
            message,
            fallback,
            source: io::Error::new(kind, message),
        }
    }
}

impl RetriableError for FetchError {
    fn should_try_next_url(&self) -> bool {
        match self {
            Self::Attempt { fallback, .. } => *fallback,
            Self::Retried { source, .. } => source.should_try_next_url(),
            Self::Clock(_) => false,
        }
    }

    fn retries(&self) -> u32 {
        match self {
            Self::Retried { retries, .. } => *retries,
            Self::Attempt { .. } | Self::Clock(_) => 0,
        }
    }

    fn into_retried(self, retries: u32, _duration: Duration) -> Self {
        Self::Retried {
            retries,
            source: Box::new(self),
        }
    }
}

fn urls() -> [DisplaySafeUrl; 2] {
    [
        "https://example.com/mirror",
        "https://example.com/canonical",
    ]
    .map(|url| DisplaySafeUrl::parse(url).expect("test URL should be valid"))
}

fn retry_policy(retries: u32) -> ExponentialBackoff {
    ExponentialBackoff::builder()
        .retry_bounds(Duration::ZERO, Duration::ZERO)
        .build_with_max_retries(retries)
}

#[tokio::test]
async fn success_stops_before_the_next_url() -> Result<()> {
    let mut attempts = Vec::new();
    let result = fetch_with_url_fallback(&urls(), retry_policy(2), "test response", async |url| {
        attempts.push(url.path().to_owned());
        Ok::<_, FetchError>("response")
    })
    .await?;

    assert_eq!(result, "response");
    assert_eq!(attempts, ["/mirror"]);
    Ok(())
}

#[tokio::test]
async fn exhausted_urls_restart_from_the_beginning() -> Result<()> {
    let mut attempts = Vec::new();
    let result = fetch_with_url_fallback(&urls(), retry_policy(1), "test response", async |url| {
        attempts.push(url.path().to_owned());
        if attempts.len() == 4 {
            Ok("response")
        } else {
            Err(FetchError::attempt(
                "response interrupted",
                true,
                io::ErrorKind::BrokenPipe,
            ))
        }
    })
    .await?;

    assert_eq!(result, "response");
    assert_eq!(attempts, ["/mirror", "/canonical", "/mirror", "/canonical"]);
    Ok(())
}

#[tokio::test]
async fn fatal_error_does_not_try_the_next_url() {
    let mut attempts = Vec::new();
    let error = fetch_with_url_fallback(&urls(), retry_policy(2), "test response", async |url| {
        attempts.push(url.path().to_owned());
        Err::<(), _>(FetchError::attempt(
            "invalid response",
            false,
            io::ErrorKind::InvalidInput,
        ))
    })
    .await
    .expect_err("the fatal error should be returned");

    assert_eq!(attempts, ["/mirror"]);
    assert_matches!(
        error,
        FetchError::Attempt { message: "invalid response", source, .. }
            if source.kind() == io::ErrorKind::InvalidInput
    );
}

#[tokio::test]
async fn transient_error_can_opt_out_of_url_fallback() -> Result<()> {
    let mut attempts = Vec::new();
    let result = fetch_with_url_fallback(&urls(), retry_policy(1), "test response", async |url| {
        attempts.push(url.path().to_owned());
        if attempts.len() == 2 {
            Ok("response")
        } else {
            Err(FetchError::attempt(
                "response interrupted",
                false,
                io::ErrorKind::BrokenPipe,
            ))
        }
    })
    .await?;

    assert_eq!(result, "response");
    assert_eq!(attempts, ["/mirror", "/mirror"]);
    Ok(())
}

#[tokio::test]
async fn exhausted_retries_preserve_the_last_error() {
    let mut attempts = Vec::new();
    let error = fetch_with_url_fallback(&urls(), retry_policy(2), "test response", async |url| {
        attempts.push(url.path().to_owned());
        let message = match url.path() {
            "/mirror" => "mirror interrupted",
            _ => "canonical interrupted",
        };
        Err::<(), _>(FetchError::attempt(
            message,
            true,
            io::ErrorKind::BrokenPipe,
        ))
    })
    .await
    .expect_err("the exhausted retry budget should return an error");

    assert_eq!(
        attempts,
        [
            "/mirror",
            "/canonical",
            "/mirror",
            "/canonical",
            "/mirror",
            "/canonical"
        ]
    );
    assert_matches!(&error, FetchError::Retried { retries: 2, .. });
    let original = error.source().expect("the original error is retained");
    assert_eq!(original.to_string(), "canonical interrupted");
    let source = original
        .source()
        .and_then(|source| source.downcast_ref::<io::Error>())
        .expect("the original I/O error is retained");
    assert_eq!(source.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(source.to_string(), "canonical interrupted");
}
