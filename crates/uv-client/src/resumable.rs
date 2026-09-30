use std::io;

use bytes::Bytes;
use futures::stream::{self, BoxStream};
use futures::{StreamExt, TryStreamExt};
use http_content_range::ContentRange;
use reqwest::header::{
    ACCEPT_ENCODING, ACCEPT_RANGES, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG,
    HeaderValue, IF_RANGE, RANGE,
};
use reqwest::{Response, StatusCode};
use tracing::debug;
use url::Url;

use uv_redacted::DisplaySafeUrl;

use crate::{RedirectClientWithMiddleware, RetryState};

/// Read a complete response, resuming an interrupted body when the server supplies a strong ETag.
///
/// The initial request must use identity encoding and have its middleware retries recorded in
/// `retry_state`. Continuations consume the same retry budget. A resumed response must describe the
/// same representation and begin exactly after the last byte yielded to the caller.
pub fn resumable_bytes_stream<'a>(
    response: Response,
    client: &'a RedirectClientWithMiddleware,
    url: &'a DisplaySafeUrl,
    retry_state: &'a mut RetryState,
) -> BoxStream<'a, reqwest_middleware::Result<Bytes>> {
    let representation = Representation::from_response(&response);
    let state = Download {
        body: response.bytes_stream().boxed(),
        representation,
        offset: 0,
        range_end: None,
        resumed_at: None,
        client,
        url,
        retry_state,
    };

    stream::try_unfold(state, async |mut state| {
        loop {
            match state.body.try_next().await {
                Ok(Some(bytes)) => {
                    let Some(offset) = state.offset.checked_add(bytes.len() as u64) else {
                        return Err(invalid_response("Download exceeds the maximum size"));
                    };
                    if state.range_end.is_some_and(|end| offset > end) {
                        return Err(invalid_response(
                            "Range response exceeded its declared size",
                        ));
                    }
                    state.offset = offset;
                    return Ok(Some((bytes, state)));
                }
                Ok(None) => {
                    let Some(end) = state.range_end else {
                        return Ok(None);
                    };
                    if state.offset != end {
                        return Err(invalid_response(
                            "Range response ended before its declared size",
                        ));
                    }
                    if state
                        .representation
                        .as_ref()
                        .is_some_and(|representation| state.offset == representation.size)
                    {
                        return Ok(None);
                    }
                    // A server may satisfy only part of a range. Request the remaining bytes
                    // without charging a successfully completed response as a retry.
                    state.resume().await?;
                }
                Err(err) => {
                    if !state.can_resume() {
                        return Err(err.into());
                    }
                    let Some(backoff) = state.retry_state.should_retry(&err, 0) else {
                        return Err(err.into());
                    };
                    state.retry_state.sleep_backoff(backoff).await;
                    state.resume().await?;
                }
            }
        }
    })
    .boxed()
}

struct Download<'a> {
    body: BoxStream<'static, Result<Bytes, reqwest::Error>>,
    representation: Option<Representation>,
    offset: u64,
    range_end: Option<u64>,
    resumed_at: Option<u64>,
    client: &'a RedirectClientWithMiddleware,
    url: &'a DisplaySafeUrl,
    retry_state: &'a mut RetryState,
}

impl Download<'_> {
    fn can_resume(&self) -> bool {
        self.representation
            .as_ref()
            .is_some_and(|representation| self.offset > 0 && self.offset < representation.size)
            && self.resumed_at.is_none_or(|offset| self.offset > offset)
    }

    async fn resume(&mut self) -> reqwest_middleware::Result<()> {
        let Some(representation) = self.representation.as_ref() else {
            return Err(invalid_response("Response cannot be resumed"));
        };
        debug!("Resuming download of {} at byte {}", self.url, self.offset);
        let response = self
            .retry_state
            .send(
                self.client
                    .get(Url::from(self.url.clone()))
                    .header(ACCEPT_ENCODING, HeaderValue::from_static("identity"))
                    .header(RANGE, format!("bytes={}-", self.offset))
                    .header(IF_RANGE, representation.etag.clone()),
            )
            .await?
            .error_for_status()?;
        let Some(end) = representation.range_end(&response, self.offset) else {
            return Err(invalid_response(
                "Invalid response to download continuation",
            ));
        };
        self.range_end = Some(end);
        self.resumed_at = Some(self.offset);
        self.body = response.bytes_stream().boxed();
        Ok(())
    }
}

struct Representation {
    etag: HeaderValue,
    size: u64,
}

impl Representation {
    fn from_response(response: &Response) -> Option<Self> {
        if response.status() != StatusCode::OK
            || response.headers().get(ACCEPT_RANGES)? != "bytes"
            || !identity_encoded(response)
        {
            return None;
        }
        let etag = response.headers().get(ETAG)?;
        let value = etag.as_bytes();
        if value.len() < 2 || value.first() != Some(&b'"') || value.last() != Some(&b'"') {
            return None;
        }
        let size = response
            .headers()
            .get(CONTENT_LENGTH)?
            .to_str()
            .ok()?
            .parse()
            .ok()?;
        Some(Self {
            etag: etag.clone(),
            size,
        })
    }

    fn range_end(&self, response: &Response, offset: u64) -> Option<u64> {
        if response.status() != StatusCode::PARTIAL_CONTENT
            || response.headers().get(ETAG)? != &self.etag
            || !identity_encoded(response)
        {
            return None;
        }
        let range = ContentRange::parse(response.headers().get(CONTENT_RANGE)?.to_str().ok()?)?;
        let (first, last, size) = match range {
            ContentRange::Bytes(range) => {
                (range.first_byte, range.last_byte, range.complete_length)
            }
            ContentRange::UnboundBytes(range) => (range.first_byte, range.last_byte, self.size),
            ContentRange::Unsatisfied(_) => return None,
        };
        if first != offset || first > last || last >= size || size != self.size {
            return None;
        }
        let end = last + 1;
        if let Some(length) = response.headers().get(CONTENT_LENGTH)
            && length.to_str().ok()?.parse::<u64>().ok()? != end - first
        {
            return None;
        }
        Some(end)
    }
}

fn identity_encoded(response: &Response) -> bool {
    response
        .headers()
        .get(CONTENT_ENCODING)
        .is_none_or(|encoding| encoding == "identity")
}

fn invalid_response(message: &'static str) -> reqwest_middleware::Error {
    reqwest_middleware::Error::middleware(io::Error::new(io::ErrorKind::InvalidData, message))
}
