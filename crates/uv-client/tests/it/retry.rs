use std::io;
use std::time::Duration;

use anyhow::Result;
use reqwest_retry::policies::ExponentialBackoff;
use uv_client::RetryState;
use uv_redacted::DisplaySafeUrl;

fn retry_state(maximum: u32) -> Result<RetryState> {
    let policy = ExponentialBackoff::builder()
        .retry_bounds(Duration::ZERO, Duration::ZERO)
        .build_with_max_retries(maximum);
    Ok(RetryState::start(
        policy,
        DisplaySafeUrl::parse("https://example.org/archive")?,
    ))
}

#[test]
fn outer_retries_exhaust_budget() -> Result<()> {
    let error = io::Error::new(io::ErrorKind::BrokenPipe, "injected response body error");

    for maximum in [0, 1, 3] {
        let mut state = retry_state(maximum)?;
        for _ in 0..maximum {
            assert!(state.should_retry(&error, 0).is_some());
        }
        assert_eq!(state.should_retry(&error, 0), None);
    }
    Ok(())
}

#[test]
fn middleware_retries_share_budget() -> Result<()> {
    let error = io::Error::new(io::ErrorKind::BrokenPipe, "injected response body error");

    for (maximum, attempts) in [
        (0, &[(0, false)][..]),
        (1, &[(1, false)][..]),
        (3, &[(2, true), (0, false)][..]),
        (3, &[(1, true), (1, false)][..]),
        (3, &[(0, true), (1, true), (0, false)][..]),
        (3, &[(3, false)][..]),
        (3, &[(4, false)][..]),
    ] {
        let mut state = retry_state(maximum)?;
        for &(middleware_retries, retry) in attempts {
            assert_eq!(
                state.should_retry(&error, middleware_retries).is_some(),
                retry,
                "maximum={maximum}, middleware_retries={middleware_retries}",
            );
        }
    }
    Ok(())
}
