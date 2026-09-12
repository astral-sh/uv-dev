/*!
A focused implementation of HTTP cache semantics.

This implementation uses:

* RFCs 9110 and 9111.
* Guidance from the `http-cache-semantics` crate, with a separate implementation.
* Zero-copy deserialization to avoid unnecessary work when a cached response is fresh.

# Flow

HTTP caching avoids network requests. When a request is necessary, it can reduce
bandwidth use. The `Cache-Control` header controls caching on requests and
responses. Its directives include:

* `no-store`, which prevents caching.
* `no-cache`, which requires revalidation before reuse.
* `max-stale`, which permits a cached response to be stale.
* `max-age`, which sets how long a response remains fresh.

The `max-age` directive is especially useful in these cases:

* PyPI responses use `max-age=600`. For 10 minutes, uv can reuse cached package
  versions without contacting PyPI.
* Wheel files usually do not change. Servers can give them a long `max-age`, so
  uv rarely needs to revalidate a cached wheel.

A response becomes stale when its age exceeds `max-age`. uv usually does not
return stale responses. A request can override this with `max-stale`, but uv
does not currently use that directive. Instead, uv can send a revalidation
request.

A revalidation request includes metadata from the cached response, usually an
entity tag or `ETag`. The server compares that metadata with its current
resource. If the resource still matches, the server can return HTTP 304 NOT
MODIFIED without a response body. uv can then reuse the cached response.
However, uv must update its stored `CachePolicy` because the 304 response can
include new caching metadata, such as an updated `Age` header.

# Scope

This module implements a private client cache for uv data. Unlike
`http-cache-semantics`, it does not store every request and response header.
It stores only the information needed to make HTTP caching decisions.

For example, a `Vary` response header lists request headers that affect a
response. A new request can reuse the cached response only when those header
values match the original request. The cache stores only the listed headers.

Because uv is not a proxy, this module does not implement proxy-specific cache
rules.

# Zero-copy deserialization

The fast path reuses a fresh response without sending a revalidation request.
It also avoids deserializing a `CachePolicy`. The cache still needs policy data
to determine whether a response is fresh.

Each cache type implements the `rkyv` traits. This permits cached bytes to
become a `rkyvutil::OwnedArchive<CachePolicy>` after a short validation step.
The archive provides an `ArchivedCachePolicy`, which
`derive(rkyv::Archive)` creates. All HTTP cache decisions therefore use
`ArchivedCachePolicy`, not `CachePolicy`.

Archived fields use archived types. For example, a `Vec` becomes an
[`rkyv::vec::ArchivedVec`], and a `String` becomes an
[`rkyv::string::ArchivedString`]. These types support the read-only operations
that cache decisions require.

When a caller has a `CachePolicy`, `CachePolicy::to_archived` serializes it into
an `OwnedArchive<CachePolicy>`. The archive dereferences to
`ArchivedCachePolicy`. This extra work occurs only on the slower path, when uv
must send an HTTP request.

[`rkyv::vec::ArchivedVec`]: https://docs.rs/rkyv/0.7.43/rkyv/vec/struct.ArchivedVec.html
[`rkyv::string::ArchivedString`]: https://docs.rs/rkyv/0.7.43/rkyv/string/struct.ArchivedString.html

# Additional reading

* Short introduction to `Cache-Control`: <https://csswizardry.com/2019/03/cache-control-for-civilians/>
* Caching best practices: <https://jakearchibald.com/2016/caching-best-practices/>
* Overview of HTTP caching: <https://developer.mozilla.org/en-US/docs/Web/HTTP/Caching>
* MDN docs for `Cache-Control`: <https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Cache-Control>
* The 1997 RFC for HTTP 1.1: <https://www.rfc-editor.org/rfc/rfc2068#section-13>
* The 1999 update to HTTP 1.1: <https://www.rfc-editor.org/rfc/rfc2616.html#section-13>
* The "stale content" cache-control extension: <https://httpwg.org/specs/rfc5861.html>
* HTTP 1.1 caching (superseded by RFC 9111): <https://httpwg.org/specs/rfc7234.html>
* The "immutable" cache-control extension: <https://httpwg.org/specs/rfc8246.html>
* HTTP semantics (If-None-Match, etc.): <https://www.rfc-editor.org/rfc/rfc9110#section-8.8.3>
* HTTP caching (obsoletes RFC 7234): <https://www.rfc-editor.org/rfc/rfc9111.html>
*/

use std::time::{Duration, SystemTime};

use http::header::HeaderValue;

use crate::rkyvutil::OwnedArchive;

use self::control::CacheControl;

mod control;

/// Settings that control uv's cache behavior.
///
/// These settings cannot currently be modified. A separate type lets
/// `CachePolicyBuilder` and `CachePolicy` share the same settings.
#[derive(
    Clone,
    Debug,
    Default,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Portable,
    rkyv::Serialize,
    bytecheck::CheckBytes,
)]
// `CacheConfig` can use itself as its archived type because its fields are simple.
// Adding a field such as `Option<u8>` would require a separate archived type.
#[rkyv(as = Self)]
#[repr(C)]
struct CacheConfig {
    shared: bool,
}

/// A builder for a [`CachePolicy`].
///
/// Use a builder directly for an HTTP request that has no cached response.
/// [`CachePolicy::before_request`] also creates a builder when a cached response is stale.
///
/// The builder collects data from an HTTP request and its response to create a [`CachePolicy`].
#[derive(Debug)]
pub(crate) struct CachePolicyBuilder {
    /// The settings that control cache behavior.
    config: CacheConfig,
    /// The HTTP request data needed for future cache decisions.
    request: Request,
    /// All request headers needed to implement the `Vary` check in [RFC 9111 S4.1].
    /// Keep these headers only until the response arrives. Do not store them in [`CachePolicy`].
    ///
    /// Sending the request transfers ownership. The response then determines which request
    /// headers must be cached, so this copy must remain available until that response arrives.
    ///
    /// [RFC 9111 S4.1]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.1
    request_headers: http::HeaderMap,
}

impl CachePolicyBuilder {
    /// Create a cache policy builder from an HTTP request.
    pub(crate) fn new(request: &reqwest::Request) -> Self {
        let config = CacheConfig::default();
        let request_headers = request.headers().clone();
        let request = Request::from(request);
        Self {
            config,
            request,
            request_headers,
        }
    }

    /// Create a policy from the response to the builder's original request.
    pub(crate) fn build(self, response: &reqwest::Response) -> CachePolicy {
        let vary = Vary::from_request_response_headers(&self.request_headers, response.headers());
        CachePolicy {
            config: self.config,
            request: self.request,
            response: Response::from(response),
            vary,
        }
    }
}

/// The data needed to implement HTTP caching in uv.
///
/// Store a cache policy with its cached data. The policy contains the information needed to detect
/// stale responses and send revalidation requests.
///
/// This type does not implement every HTTP cache rule. In particular, it excludes proxy caching.
#[derive(Debug, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
#[rkyv(derive(Debug))]
pub(crate) struct CachePolicy {
    /// The settings that control cache behavior.
    config: CacheConfig,
    /// The HTTP request data needed for future cache decisions.
    request: Request,
    /// The HTTP response data needed for future cache decisions.
    response: Response,
    /// Header names from the cached response's `Vary` header and values from the original request.
    /// Use these pairs to determine whether a new request can reuse the cached response.
    vary: Vary,
}

impl CachePolicy {
    /// Convert this policy to an owned archive.
    ///
    /// Cache decisions use archived types, so archive the policy before evaluating it.
    ///
    /// This conversion adds a small cost when no [`ArchivedCachePolicy`] is available.
    /// That case should occur only when uv sends an HTTP request.
    pub(crate) fn to_archived(&self) -> OwnedArchive<Self> {
        // Serialization can fail only if the process runs out of memory.
        OwnedArchive::from_unarchived(self).expect("all possible values can be archived")
    }
}

impl ArchivedCachePolicy {
    /// Return whether this cached response matches a request that permits stale data.
    ///
    /// Apply only the [`Self::before_request`] conditions that produce [`BeforeRequest::NoMatch`].
    /// Skip freshness, `Vary`, and `no-cache` checks because they produce
    /// [`BeforeRequest::Stale`], which the caller accepts.
    pub(crate) fn matches_stale_request(&self, request: &reqwest::Request) -> bool {
        self.is_storable()
            && self.request.uri == request.url().as_str()
            && (request.method() == http::Method::GET || request.method() == http::Method::HEAD)
    }

    /// Determine how to handle a new request under an existing cache policy.
    /// Follow [RFC 9111 S4].
    ///
    /// This method determines whether the caller can reuse a cached response or must contact
    /// the origin server.
    ///
    /// Return one of these results:
    ///
    /// 1. The response is fresh. Return it without sending an HTTP request.
    /// 2. The response is stale. Send a revalidation request, then call
    ///    `CachePolicy::after_response` to determine whether to update the response.
    /// 3. The request does not match the cache policy. This usually means the cache loaded the
    ///    wrong policy.
    ///
    /// For a stale response, modify the request in place to prepare it for revalidation.
    ///
    /// [RFC 9111 S4]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4
    pub(crate) fn before_request(&self, request: &mut reqwest::Request) -> BeforeRequest {
        let now = SystemTime::now();
        // Reject a response that the cache cannot store.
        if !self.is_storable() {
            tracing::trace!(
                "Request {} does not match cache request {} because it isn't storable",
                request.url(),
                self.request.uri,
            );
            return BeforeRequest::NoMatch;
        }
        // "When presented with a request, a cache MUST NOT reuse a stored
        // response unless..."
        //
        // "the presented target URI and that of the stored response match,
        // and..."
        if self.request.uri != request.url().as_str() {
            tracing::trace!(
                "Request {} does not match cache URL of {}",
                request.url(),
                self.request.uri,
            );
            return BeforeRequest::NoMatch;
        }
        // "the request method associated with the stored response allows it to
        // be used for the presented request, and..."
        if request.method() != http::Method::GET && request.method() != http::Method::HEAD {
            tracing::trace!(
                "Method {:?} for request {} is not supported by this cache",
                request.method(),
                request.url(),
            );
            return BeforeRequest::NoMatch;
        }
        // "Request header fields nominated by the stored response (if any)
        // match those presented, and..."
        //
        // Require revalidation if the request does not match the cached `Vary` header.
        if !self.vary.matches(request.headers()) {
            tracing::trace!(
                "Request {} does not match cached request because of the 'Vary' header",
                request.url(),
            );
            self.set_revalidation_headers(request);
            return BeforeRequest::Stale(self.new_cache_policy_builder(request));
        }
        // "the stored response does not contain the no-cache directive, unless
        // it is successfully validated, and..."
        if self.response.headers.cc.no_cache {
            self.set_revalidation_headers(request);
            return BeforeRequest::Stale(self.new_cache_policy_builder(request));
        }
        // "the stored response is one of the following: ..."
        //
        // "fresh, or..."
        // "allowed to be served stale, or..."
        if self.is_fresh(now, request) {
            return BeforeRequest::Fresh;
        }
        // "successfully validated."
        //
        // The caller must send a revalidation request.
        self.set_revalidation_headers(request);
        BeforeRequest::Stale(self.new_cache_policy_builder(request))
    }

    /// Handle a response to a request that can revalidate cached data.
    /// Follow [RFC 9111 S4.3.3] and [RFC 9111 S4.3.4].
    ///
    /// Use the policy builder from `CachePolicy::before_request` and the origin server's response.
    /// Call this method after [`BeforeRequest::Stale`], although new requests are also supported.
    ///
    /// [`AfterResponse::NotModified`] means the cached response remains fresh.
    /// [`AfterResponse::Modified`] means the caller must cache the new response.
    ///
    /// Update the cache with the new policy in both cases.
    ///
    /// [RFC 9111 S4.3.3]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.3.3
    /// [RFC 9111 S4.3.4]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.3.4
    pub(crate) fn after_response(
        &self,
        cache_policy_builder: CachePolicyBuilder,
        response: &reqwest::Response,
    ) -> AfterResponse {
        let mut new_policy = cache_policy_builder.build(response);
        if self.is_modified(&new_policy) {
            AfterResponse::Modified(new_policy)
        } else {
            new_policy.response.status = self.response.status.into();
            AfterResponse::NotModified(new_policy)
        }
    }

    fn is_modified(&self, new_policy: &CachePolicy) -> bool {
        // From [RFC 9111 S4.3.3],
        //
        // "A 304 (Not Modified) response status code indicates that the stored
        // response can be updated and reused"
        //
        // If the status is not 304, the origin server treats the cached response as stale.
        //
        // [RFC 9111 S4.3.3]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.3.3
        if new_policy.response.status != 304 {
            tracing::trace!(
                "Resource is modified because status is {:?} and not 304",
                new_policy.response.status
            );
            return true;
        }
        // Check that the `ETag` validators match, as required by [RFC 9111 S4.3.4].
        //
        // [RFC 9111 S4.3.4]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.3.4
        if let Some(old_etag) = self.response.headers.etag.as_ref() {
            if let Some(new_etag) = new_policy.response.headers.etag.as_ref() {
                // Weak validators are not supported. Match only if both validators are strong.
                if !old_etag.weak && !new_etag.weak && old_etag.value == new_etag.value {
                    tracing::trace!(
                        "Resource is not modified because old and new etag values ({:?}) match",
                        new_etag.value,
                    );
                    return false;
                }
            }
        }
        // Check that the `Last-Modified` validators match, as required by [RFC 9111 S4.3.4].
        //
        // [RFC 9111 S4.3.4]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.3.4
        if let Some(old_last_modified) = self.response.headers.last_modified_unix_timestamp.as_ref()
        {
            if let Some(new_last_modified) = new_policy
                .response
                .headers
                .last_modified_unix_timestamp
                .as_ref()
            {
                if old_last_modified == new_last_modified {
                    tracing::trace!(
                        "Resource is not modified because modified times ({new_last_modified:?}) match",
                    );
                    return false;
                }
            }
        }
        // If neither response has validators, [RFC 9111 S4.3.4] permits reuse after HTTP 304.
        //
        // [RFC 9111 S4.3.4]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.3.4
        if self.response.headers.etag.is_none()
            && new_policy.response.headers.etag.is_none()
            && self.response.headers.last_modified_unix_timestamp.is_none()
            && new_policy
                .response
                .headers
                .last_modified_unix_timestamp
                .is_none()
        {
            tracing::trace!(
                "Resource is not modified because there are no etags or last modified \
                 timestamps, so we assume the 304 status is correct",
            );
            return false;
        }
        true
    }

    /// Add the headers needed to revalidate the request under [RFC 9111 S4.3.1].
    /// If the content has not changed, the origin server can return HTTP 304 NOT MODIFIED.
    /// This avoids sending the response body again and permits reuse of the cached response.
    ///
    /// Add a strong `ETag` validator when the cached response has one.
    /// Preserve any `ETag` validator that the request already contains.
    ///
    /// Preserve an existing `If-Modified-Since` header.
    /// If that header is absent, add it when the cached response has a valid `Last-Modified`
    /// header.
    ///
    /// [RFC 9111 S4.3.1]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.3.1
    fn set_revalidation_headers(&self, request: &mut reqwest::Request) {
        // Send the stored `ETag` in `If-None-Match`, as required by [RFC 9110 S13.1.2] and
        // [RFC 9111 S4.3.1]. If a tag matches, the server can return HTTP 304.
        //
        // [RFC 9110 S13.1.2]: https://www.rfc-editor.org/rfc/rfc9110#section-13.1.2
        // [RFC 9111 S4.3.1]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.3.1
        if let Some(etag) = self.response.headers.etag.as_ref() {
            // Do not use weak validation because it can accept changed content.
            // RFC 9110 S13.1.2 permits weak entity tags to validate changed representation data.
            if !etag.weak {
                if let Ok(header) = HeaderValue::from_bytes(&etag.value) {
                    request.headers_mut().append("if-none-match", header);
                }
            }
        }
        // Set `If-Modified-Since` under [RFC 9110 S13.1.3] and [RFC 9111 S4.3.1].
        // This provides a fallback if the server does not support `If-None-Match`.
        //
        // [RFC 9110 S13.1.3]: https://www.rfc-editor.org/rfc/rfc9110#section-13.1.3
        // [RFC 9111 S4.3.1]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.3.1
        if !request.headers().contains_key("if-modified-since") {
            if let Some(&last_modified_unix_timestamp) =
                self.response.headers.last_modified_unix_timestamp.as_ref()
            {
                if let Some(last_modified) =
                    unix_timestamp_to_header(last_modified_unix_timestamp.into())
                {
                    request
                        .headers_mut()
                        .insert("if-modified-since", last_modified);
                }
            }
        }
    }

    /// Return `true` if [RFC 9111 S3] permits the response to be cached.
    ///
    /// [RFC 9111 S3]: https://www.rfc-editor.org/rfc/rfc9111.html#section-3
    pub(crate) fn is_storable(&self) -> bool {
        // Without other signals, cache only status codes that [RFC 9110 S15.1] treats as cacheable.
        //
        // [RFC 9110 S15.1]: https://www.rfc-editor.org/rfc/rfc9110#section-15.1
        const HEURISTICALLY_CACHEABLE_STATUS_CODES: &[u16] =
            &[200, 203, 204, 206, 300, 301, 308, 404, 405, 410, 414, 501];

        // Follow the order of the rules in RFC 9111 S3.

        // "the request method is understood by the cache"
        //
        // Support only GET and HEAD requests.
        if !matches!(
            self.request.method,
            ArchivedMethod::Get | ArchivedMethod::Head
        ) {
            tracing::trace!(
                "Response from {} is not storable because of the request method {:?}",
                self.request.uri,
                self.request.method
            );
            return false;
        }
        // "the response status code is final"
        //
        // Reject non-final status codes before checking additional restrictions.
        if !self.response.has_final_status() {
            tracing::trace!(
                "Response from {} is not storable because it has \
                a non-final status code {:?}",
                self.request.uri,
                self.response.status,
            );
            return false;
        }
        // "if the response status code is 206 or 304, or the must-understand
        // cache directive (see Section 5.2.2.3) is present: the cache
        // understands the response status code"
        //
        // The cache does not support `must-understand` or partial content (206).
        // Do not cache a 304 response itself.
        if self.response.status == 206 || self.response.status == 304 {
            tracing::trace!(
                "Response from {} is not storable because it has \
                an unsupported status code {:?}",
                self.request.uri,
                self.response.status,
            );
            return false;
        }
        // "The no-store request directive indicates that a cache MUST NOT
        // store any part of either this request or any response to it."
        //
        // RFC 9111 S5.2.1.5 defines this rule separately from S3.
        if self.request.headers.cc.no_store {
            tracing::trace!(
                "Response from {} is not storable because its request has \
                 a 'no-store' cache-control directive",
                self.request.uri,
            );
            return false;
        }
        // "the no-store cache directive is not present in the response"
        if self.response.headers.cc.no_store {
            tracing::trace!(
                "Response from {} is not storable because it has \
                 a 'no-store' cache-control directive",
                self.request.uri,
            );
            return false;
        }
        // "if the cache is shared ..."
        if self.config.shared {
            // "if the cache is shared: the private response directive is either
            // not present or allows a shared cache to store a modified response"
            //
            // The cache does not support `private` directives that remove selected response
            // headers before shared caching.
            if self.response.headers.cc.private {
                tracing::trace!(
                    "Response from {} is not storable because this is a shared \
                     cache and has a 'private' cache-control directive",
                    self.request.uri,
                );
                return false;
            }
            // "if the cache is shared: the Authorization header field is not
            // present in the request or a response directive is present that
            // explicitly allows shared caching"
            if self.request.headers.authorization && !self.allows_authorization_storage() {
                tracing::trace!(
                    "Response from {} is not storable because this is a shared \
                     cache and the request has an 'Authorization' header set and \
                     the response has indicated that caching requests with an \
                     'Authorization' header is allowed",
                    self.request.uri,
                );
                return false;
            }
        }

        // "the response contains at least one of the following ..."
        //
        // "a public response directive"
        if self.response.headers.cc.public {
            tracing::trace!(
                "Response from {} is storable because it has \
                 a 'public' cache-control directive",
                self.request.uri,
            );
            return true;
        }
        // "a private response directive, if the cache is not shared"
        if !self.config.shared && self.response.headers.cc.private {
            tracing::trace!(
                "Response from {} is storable because this is a shared cache \
                 and has a 'private' cache-control directive",
                self.request.uri,
            );
            return true;
        }
        // "an Expires header field"
        if self.response.headers.expires_unix_timestamp.is_some() {
            tracing::trace!(
                "Response from {} is storable because it has an \
                 'Expires' header set",
                self.request.uri,
            );
            return true;
        }
        // "a max-age response directive"
        if self.response.headers.cc.max_age_seconds.is_some() {
            tracing::trace!(
                "Response from {} is storable because it has an \
                 'max-age' cache-control directive",
                self.request.uri,
            );
            return true;
        }
        // "if the cache is shared: an s-maxage response directive"
        if self.config.shared && self.response.headers.cc.s_maxage_seconds.is_some() {
            tracing::trace!(
                "Response from {} is storable because this is a shared cache \
                 and has a 's-maxage' cache-control directive",
                self.request.uri,
            );
            return true;
        }
        // "a cache extension that allows it to be cached"
        // The cache does not support extensions.
        //
        // "a status code that is defined as heuristically cacheable"
        if HEURISTICALLY_CACHEABLE_STATUS_CODES.contains(&self.response.status.into()) {
            tracing::trace!(
                "Response from {} is storable because it has a \
                 heuristically cacheable status code {:?}",
                self.request.uri,
                self.response.status,
            );
            return true;
        }
        tracing::trace!(
            "Response from {} is not storable because it does not meet any \
             of the necessary criteria (e.g., it doesn't have an 'Expires' \
             header set or a 'max-age' cache-control directive)",
            self.request.uri,
        );
        false
    }

    /// Return `true` if [RFC 9111 S3.5] permits caching a request with an `Authorization` header.
    ///
    /// [RFC 9111 S3.5]: https://www.rfc-editor.org/rfc/rfc9111.html#section-3.5
    fn allows_authorization_storage(&self) -> bool {
        self.response.headers.cc.must_revalidate
            || self.response.headers.cc.public
            || self.response.headers.cc.s_maxage_seconds.is_some()
    }

    /// Return `true` if the response is fresh under [RFC 9111 S4.2].
    /// Revalidate stale responses with the origin server.
    ///
    /// [RFC 9111 S4.2]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.2
    fn is_fresh(&self, now: SystemTime, request: &reqwest::Request) -> bool {
        let freshness_lifetime = self.freshness_lifetime().as_secs();
        let age = self.age(now).as_secs();

        // Under RFC 8246, `immutable` prevents a normal reload from sending a revalidation request.
        // Contact the origin server only after the cached response exceeds its freshness lifetime.
        //
        // A forced reload should override this rule, but this implementation does not support one.
        // Ignore request directives that would otherwise require revalidation.
        //
        // [RFC 8246]: https://httpwg.org/specs/rfc8246.html
        if !self.response.headers.cc.immutable {
            let reqcc = request
                .headers()
                .get_all("cache-control")
                .iter()
                .collect::<CacheControl>();

            // Honor the request's `no-cache` directive under [RFC 9111 S5.2.1.4].
            //
            // [RFC 9111 S5.2.1.4]: https://www.rfc-editor.org/rfc/rfc9111.html#section-5.2.1.4
            if reqcc.no_cache {
                tracing::trace!(
                    "Request to {} does not have a fresh cache entry because \
                 it has a 'no-cache' cache-control directive",
                    request.url(),
                );
                return false;
            }

            // Honor the request's `max-age` directive under [RFC 9111 S5.2.1.1].
            //
            // [RFC 9111 S5.2.1.1]: https://www.rfc-editor.org/rfc/rfc9111.html#section-5.2.1.1
            if let Some(&max_age) = reqcc.max_age_seconds.as_ref() {
                if age > max_age {
                    tracing::trace!(
                        "Request to {} does not have a fresh cache entry because \
                     the cached response's age is {} seconds and the max age \
                     allowed by the request is {} seconds",
                        request.url(),
                        age,
                        max_age,
                    );
                    return false;
                }
            }

            // Honor `min-fresh` under [RFC 9111 S5.2.1.3]. The response must remain fresh for at
            // least the requested time.
            //
            // [RFC 9111 S5.2.1.3]: https://www.rfc-editor.org/rfc/rfc9111.html#section-5.2.1.3
            if let Some(&min_fresh) = reqcc.min_fresh_seconds.as_ref() {
                let time_to_live = freshness_lifetime.saturating_sub(unix_timestamp(now));
                if time_to_live < min_fresh {
                    tracing::trace!(
                        "Request to {} does not have a fresh cache entry because \
                     the request set a 'min-fresh' cache-control directive, \
                     and its time-to-live is {} seconds but it needs to be \
                     at least {} seconds",
                        request.url(),
                        time_to_live,
                        min_fresh,
                    );
                    // S5.2.1.3 does not permit `max-stale` to override this rule.
                    return false;
                }
            }
        }
        // RFC 9111 S4.2 defines freshness as
        // `freshness_lifetime > current_age`, so equality is stale.
        //
        // [RFC 9111 S4.2]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.2
        if age >= freshness_lifetime {
            let allows_stale = self.allows_stale(now);
            if !allows_stale {
                tracing::trace!(
                    "Request to {} does not have a fresh cache entry because \
                     its age is {} seconds, it is greater than or equal to the \
                     freshness lifetime of {} seconds and stale cached responses \
                     are not allowed",
                    request.url(),
                    age,
                    freshness_lifetime,
                );
                return false;
            }
        }
        true
    }

    /// Return `true` if [RFC 9111 S4.2.4] permits a stale response.
    ///
    /// [RFC 9111 S4.2.4]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.2.4
    fn allows_stale(&self, now: SystemTime) -> bool {
        // Under [RFC 9111 S5.2.2.2], `must-revalidate` requires the cache to contact the server
        // before it reuses a stale response. Assume `must-revalidate` takes precedence over
        // `max-stale` because RFC 9111 does not define their interaction.
        //
        // [RFC 9111 S5.2.2.2]: https://www.rfc-editor.org/rfc/rfc9111.html#section-5.2.2.2
        if self.response.headers.cc.must_revalidate {
            tracing::trace!(
                "Request to {} has a cached response that does not \
                 permit staleness because the response has a 'must-revalidate' \
                 cache-control directive set",
                self.request.uri,
            );
            return false;
        }
        if let Some(&max_stale) = self.request.headers.cc.max_stale_seconds.as_ref() {
            // Under [RFC 9111 S5.2.1.2], `max-stale` permits responses that exceed their freshness
            // lifetime by no more than the specified threshold.
            //
            // [RFC 9111 S5.2.1.2]: https://www.rfc-editor.org/rfc/rfc9111.html#section-5.2.1.2
            let stale_amount = self
                .age(now)
                .as_secs()
                .saturating_sub(self.freshness_lifetime().as_secs());
            if stale_amount <= max_stale.into() {
                tracing::trace!(
                    "Request to {} has a cached response that allows staleness \
                     in this case because the stale amount is {} seconds and the \
                     'max-stale' cache-control directive set by the cached request \
                     is {} seconds",
                    self.request.uri,
                    stale_amount,
                    max_stale,
                );
                return true;
            }
        }
        // Under [RFC 9111 S4.2.4], use stale responses only when explicitly permitted:
        //
        // "A cache MUST NOT generate a stale response unless it is
        // disconnected or doing so is explicitly permitted by the client or
        // origin server..."
        //
        // [RFC 9111 S4.2.4]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.2.4
        tracing::trace!(
            "Request to {} has a cached response that does not allow staleness",
            self.request.uri,
        );
        false
    }

    /// Return the age of the HTTP response under [RFC 9111 S4.2.3].
    ///
    /// Response age measures the time since the origin server created the response.
    /// Compare this age with the freshness lifetime to determine whether the response is stale.
    ///
    /// [RFC 9111 S4.2.3]: https://www.rfc-editor.org/rfc/rfc9111.html#name-calculating-age
    fn age(&self, now: SystemTime) -> Duration {
        // RFC 9111 S4.2.3
        let apparent_age =
            u64::from(self.response.unix_timestamp).saturating_sub(self.response.header_date());
        let response_delay = u64::from(self.response.unix_timestamp)
            .saturating_sub(self.request.unix_timestamp.into());
        let corrected_age_value = self.response.header_age().saturating_add(response_delay);
        let corrected_initial_age = apparent_age.max(corrected_age_value);
        let resident_age = unix_timestamp(now).saturating_sub(self.response.unix_timestamp.into());
        let current_age = corrected_initial_age.saturating_add(resident_age);
        Duration::from_secs(current_age)
    }

    /// Return how long a response remains fresh under [RFC 9111 S4.2.1].
    ///
    /// If the response does not indicate a freshness lifetime, return `0`.
    /// A zero lifetime means the response is always stale.
    ///
    /// [RFC 9111 S4.2.1]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.2.1
    fn freshness_lifetime(&self) -> Duration {
        if self.config.shared {
            if let Some(&s_maxage) = self.response.headers.cc.s_maxage_seconds.as_ref() {
                let duration = Duration::from_secs(s_maxage.into());
                tracing::trace!(
                    "Freshness lifetime found via shared \
                     cache-control max age setting: {duration:?}"
                );
                return duration;
            }
        }
        if let Some(&max_age) = self.response.headers.cc.max_age_seconds.as_ref() {
            let duration = Duration::from_secs(max_age.into());
            tracing::trace!(
                "Freshness lifetime found via cache-control max age setting: {duration:?}"
            );
            return duration;
        }
        if let Some(&expires) = self.response.headers.expires_unix_timestamp.as_ref() {
            let duration =
                Duration::from_secs(u64::from(expires).saturating_sub(self.response.header_date()));
            tracing::trace!("Freshness lifetime found via expires header: {duration:?}");
            return duration;
        }
        if self.response.headers.last_modified_unix_timestamp.is_some() {
            // The previous heuristic used 10% of the interval between `Last-Modified` and `Date`.
            //
            // That interval can produce a long freshness lifetime and cache responses too
            // aggressively[1].
            //
            // Use the same 600-second lifetime as PyPI because uv mainly accesses package indexes.
            //
            // Indexes should instead provide a `Cache-Control` or `Expires` header.
            //
            // [1]: https://github.com/astral-sh/uv/issues/5351#issuecomment-2260588764
            let duration = Duration::from_mins(10);
            tracing::trace!(
                "Freshness lifetime heuristically assumed \
                 because of presence of last-modified header: {duration:?}"
            );
            return duration;
        }
        // Without a freshness indicator, treat the response as stale.
        tracing::trace!("Could not determine freshness lifetime, assuming none exists");
        Duration::ZERO
    }

    fn new_cache_policy_builder(&self, request: &reqwest::Request) -> CachePolicyBuilder {
        let request_headers = request.headers().clone();
        CachePolicyBuilder {
            config: self.config.clone(),
            request: Request::from(request),
            request_headers,
        }
    }
}

/// The result of calling [`CachePolicy::before_request`].
///
/// This result tells the caller whether the cached response is fresh, stale, or unrelated.
#[derive(Debug)]
#[expect(clippy::large_enum_variant)]
pub(crate) enum BeforeRequest {
    /// The response is fresh and can be returned without an HTTP request.
    Fresh,
    /// The response is stale. Send a revalidation request, then call
    /// `CachePolicy::after_response` to determine whether the cached response must be updated.
    Stale(CachePolicyBuilder),
    /// The request does not match the cache policy. This usually indicates an incorrect policy.
    NoMatch,
}

/// The result of calling [`CachePolicy::after_response`].
///
/// [`AfterResponse::NotModified`] means revalidation succeeded.
/// [`AfterResponse::Modified`] means the cached response must be updated.
#[derive(Debug)]
pub(crate) enum AfterResponse {
    /// The cached response is still fresh.
    NotModified(CachePolicy),
    /// The cached response is invalid and must be updated with the new response.
    Modified(CachePolicy),
}

#[derive(Debug, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
#[rkyv(derive(Debug))]
struct Request {
    uri: String,
    method: Method,
    headers: RequestHeaders,
    unix_timestamp: u64,
}

impl<'a> From<&'a reqwest::Request> for Request {
    fn from(from: &'a reqwest::Request) -> Self {
        Self {
            uri: from.url().to_string(),
            method: Method::from(from.method()),
            headers: RequestHeaders::from(from.headers()),
            unix_timestamp: unix_timestamp(SystemTime::now()),
        }
    }
}

#[derive(Debug, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
#[rkyv(derive(Debug))]
struct RequestHeaders {
    /// The cache directives from the `Cache-Control` header.
    cc: CacheControl,
    /// Whether an `Authorization` header is present. Do not store the header value.
    authorization: bool,
}

impl<'a> From<&'a http::HeaderMap> for RequestHeaders {
    fn from(from: &'a http::HeaderMap) -> Self {
        Self {
            cc: from.get_all("cache-control").iter().collect(),
            authorization: from.contains_key("authorization"),
        }
    }
}

/// The HTTP method used on a request.
///
/// Treat methods that cannot produce cached responses as unrecognized.
#[derive(Debug, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
#[rkyv(derive(Debug))]
#[repr(u8)]
enum Method {
    Get,
    Head,
    Unrecognized,
}

impl<'a> From<&'a http::Method> for Method {
    fn from(from: &'a http::Method) -> Self {
        if from == http::Method::GET {
            Self::Get
        } else if from == http::Method::HEAD {
            Self::Head
        } else {
            Self::Unrecognized
        }
    }
}

#[derive(Debug, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
#[rkyv(derive(Debug))]
struct Response {
    status: u16,
    headers: ResponseHeaders,
    unix_timestamp: u64,
}

impl ArchivedResponse {
    /// Return the response's `Age` header value under [RFC 9111 S4.2.3].
    /// Return `0` if the header is absent or invalid.
    ///
    /// This value is not the complete response age. `ArchivedCachePolicy::age` calculates that
    /// age from additional information, such as the request time.
    ///
    /// [RFC 9111 S4.2.3]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.2.3
    fn header_age(&self) -> u64 {
        self.headers
            .age_seconds
            .as_ref()
            .map(u64::from)
            .unwrap_or(0)
    }

    /// Return the response's `Date` header value under [RFC 9110 S6.6.1].
    /// If the header is absent, use the time when the response arrived.
    ///
    /// [RFC 9110 S6.6.1]: https://www.rfc-editor.org/rfc/rfc9110#section-6.6.1
    fn header_date(&self) -> u64 {
        self.headers
            .date_unix_timestamp
            .unwrap_or(self.unix_timestamp)
            .into()
    }

    /// Return `true` if the response has a final status code under [RFC 9110 S15].
    ///
    /// [RFC 9110 S15]: https://www.rfc-editor.org/rfc/rfc9110#section-15
    fn has_final_status(&self) -> bool {
        self.status >= 200
    }
}

impl<'a> From<&'a reqwest::Response> for Response {
    fn from(from: &'a reqwest::Response) -> Self {
        Self {
            status: from.status().as_u16(),
            headers: ResponseHeaders::from(from.headers()),
            unix_timestamp: unix_timestamp(SystemTime::now()),
        }
    }
}

#[derive(Debug, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
#[rkyv(derive(Debug))]
struct ResponseHeaders {
    /// The directives from the `Cache-Control` header.
    cc: CacheControl,
    /// The `Age` header value, called `age_value` in [RFC 9111 S4.2.3].
    /// Treat an absent `Age` header as `0`.
    ///
    /// [RFC 9111 S4.2.3]: https://www.rfc-editor.org/rfc/rfc9111.html#name-calculating-age
    age_seconds: Option<u64>,
    /// The `date_value` from [RFC 9111 S4.2.3] and the `Date` header from [RFC 7231 S7.1.1.2].
    /// If the `Date` header is absent, use the response time in `Response::unix_timestamp`.
    ///
    /// [RFC 9111 S4.2.3]: https://www.rfc-editor.org/rfc/rfc9111.html#name-calculating-age
    /// [RFC 7231 S7.1.1.2]: https://httpwg.org/specs/rfc7231.html#header.date
    date_unix_timestamp: Option<u64>,
    /// The `Expires` header value from [RFC 9111 S5.3].
    /// The `max-age` and `s-maxage` cache directives take precedence over this value.
    ///
    /// If an `Expires` header contains an invalid RFC 2822 date, use `Some(0)`.
    /// This represents a time in the past and treats the response as expired.
    ///
    /// [RFC 9111 S5.3]: https://www.rfc-editor.org/rfc/rfc9111.html#section-5.3
    expires_unix_timestamp: Option<u64>,
    /// The RFC 2822 date from the `Last-Modified` header in [RFC 9110 S8.8.2].
    /// If other freshness indicators are absent, use this value under [RFC 9111 S4.2.2].
    ///
    /// [RFC 9110 S8.8.2]: https://www.rfc-editor.org/rfc/rfc9110#section-8.8.2
    /// [RFC 9111 S4.2.2]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.2.2
    last_modified_unix_timestamp: Option<u64>,
    /// The response's entity tag from [RFC 9110 S8.8.3], used for revalidation requests.
    ///
    /// [RFC 9110 S8.8.3]: https://www.rfc-editor.org/rfc/rfc9110#section-8.8.3
    etag: Option<ETag>,
}

impl<'a> From<&'a http::HeaderMap> for ResponseHeaders {
    fn from(from: &'a http::HeaderMap) -> Self {
        Self {
            cc: from.get_all("cache-control").iter().collect(),
            age_seconds: from
                .get("age")
                .and_then(|header| parse_seconds(header.as_bytes())),
            date_unix_timestamp: from
                .get("date")
                .and_then(|header| header.to_str().ok())
                .and_then(rfc2822_to_unix_timestamp),
            expires_unix_timestamp: from
                .get("expires")
                .and_then(|header| header.to_str().ok())
                .and_then(rfc2822_to_unix_timestamp),
            last_modified_unix_timestamp: from
                .get("last-modified")
                .and_then(|header| header.to_str().ok())
                .and_then(rfc2822_to_unix_timestamp),
            etag: from
                .get("etag")
                .map(|header| ETag::parse(header.as_bytes())),
        }
    }
}

#[derive(Debug, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
#[rkyv(derive(Debug))]
struct ETag {
    /// The actual `ETag` validator value.
    ///
    /// Store the response's validator in the cache policy and include it in revalidation requests.
    /// A matching validator lets the server return HTTP 304 NOT MODIFIED.
    value: Vec<u8>,
    /// When `weak` is true, this entity tag is a weak validator under [RFC 9110 S8.8.1]:
    ///
    /// "In contrast, a "weak validator" is representation metadata that might
    /// not change for every change to the representation data. This weakness
    /// might be due to limitations in how the value is calculated (e.g.,
    /// clock resolution), an inability to ensure uniqueness for all possible
    /// representations of the resource, or a desire of the resource owner to
    /// group representations by some self-determined set of equivalency rather
    /// than unique sequences of data."
    ///
    /// Weak validation is not currently supported.
    ///
    /// [RFC 9110 S8.8.1]: https://www.rfc-editor.org/rfc/rfc9110#section-8.8.1-6
    weak: bool,
}

impl ETag {
    /// Parse an `ETag` from a header value.
    ///
    /// Accept arbitrary bytes, even though [RFC 9110 S8.8.3] is more restrictive.
    ///
    /// [RFC 9110 S8.8.3]: https://www.rfc-editor.org/rfc/rfc9110#section-8.8.3
    fn parse(header_value: &[u8]) -> Self {
        let (value, weak) = if header_value.starts_with(b"W/") {
            (&header_value[2..], true)
        } else {
            (header_value, false)
        };
        Self {
            value: value.to_vec(),
            weak,
        }
    }
}

/// The `Vary` header from a cached response under [RFC 9110 S12.5.5] and [RFC 9111 S4.1].
///
/// Reuse a cached response only when the new request matches the original values of the listed
/// headers.
///
/// [RFC 9110 S12.5.5]: https://www.rfc-editor.org/rfc/rfc9110#section-12.5.5
/// [RFC 9111 S4.1]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.1
#[derive(Debug, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
#[rkyv(derive(Debug))]
struct Vary {
    fields: Vec<VaryField>,
}

impl Vary {
    /// Return a `Vary` header value that never matches a request.
    fn always_fails_to_match() -> Self {
        Self {
            fields: vec![VaryField {
                name: "*".to_string(),
                value: vec![],
            }],
        }
    }

    fn from_request_response_headers(
        request: &http::HeaderMap,
        response: &http::HeaderMap,
    ) -> Self {
        // Parse the `Vary` header under [RFC 9110 S12.5.5].
        //
        // [RFC 9110 S12.5.5]: https://www.rfc-editor.org/rfc/rfc9110#section-12.5.5
        let mut fields = vec![];
        for header in response.get_all("vary") {
            let Ok(csv) = header.to_str() else { continue };
            for header_name in csv.split(',') {
                let header_name = header_name.trim().to_ascii_lowercase();
                // A `*` matches no request. Return a `Vary` value that always fails.
                if header_name == "*" {
                    return Self::always_fails_to_match();
                }
                let value = request
                    .get(&header_name)
                    .map(|header| header.as_bytes().to_vec())
                    .unwrap_or_default();
                fields.push(VaryField {
                    name: header_name,
                    value,
                });
            }
        }
        Self { fields }
    }
}

impl ArchivedVary {
    /// Return `true` if the cached `Vary` header matches the request under [RFC 9111 S4.1].
    ///
    /// [RFC 9111 S4.1]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.1
    fn matches(&self, request_headers: &http::HeaderMap) -> bool {
        for field in self.fields.iter() {
            // A `*` anywhere means the match always fails.
            if field.name == "*" {
                return false;
            }
            let request_header_value = request_headers
                .get(field.name.as_str())
                .map_or(&b""[..], |header| header.as_bytes());
            if field.value.as_slice() != request_header_value {
                return false;
            }
        }
        true
    }
}

/// One field and value from a response's `Vary` header under [RFC 9111 S4.1].
///
/// Get the field name from the response's `Vary` header and its value from the original request.
/// A new request can reuse the cached response only if its field value matches.
///
/// [RFC 9111 S4.1]: https://www.rfc-editor.org/rfc/rfc9111.html#section-4.1
#[derive(Debug, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
#[rkyv(derive(Debug))]
struct VaryField {
    name: String,
    value: Vec<u8>,
}

fn unix_timestamp(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .expect("UNIX_EPOCH is as early as it gets")
        .as_secs()
}

fn rfc2822_to_unix_timestamp(s: &str) -> Option<u64> {
    rfc2822_to_datetime(s).and_then(|timestamp| u64::try_from(timestamp.as_second()).ok())
}

fn rfc2822_to_datetime(s: &str) -> Option<jiff::Timestamp> {
    jiff::fmt::rfc2822::DateTimeParser::new()
        .parse_timestamp(s)
        .ok()
}

fn unix_timestamp_to_header(seconds: u64) -> Option<HeaderValue> {
    unix_timestamp_to_rfc2822(seconds).and_then(|string| HeaderValue::from_str(&string).ok())
}

fn unix_timestamp_to_rfc2822(seconds: u64) -> Option<String> {
    use jiff::fmt::rfc2822::DateTimePrinter;

    unix_timestamp_to_datetime(seconds).and_then(|timestamp| {
        DateTimePrinter::new()
            .timestamp_to_rfc9110_string(&timestamp)
            .ok()
    })
}

fn unix_timestamp_to_datetime(seconds: u64) -> Option<jiff::Timestamp> {
    jiff::Timestamp::from_second(i64::try_from(seconds).ok()?).ok()
}

fn parse_seconds(value: &[u8]) -> Option<u64> {
    if !value.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(value).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::time::Duration;

    use super::*;

    fn http_request(method: http::Method, uri: &str) -> reqwest::Request {
        reqwest::Request::new(method, uri.parse().unwrap())
    }

    fn archived_cache_policy(
        request: &reqwest::Request,
        response: http::response::Builder,
    ) -> OwnedArchive<CachePolicy> {
        let response = reqwest::Response::from(response.body(Vec::new()).unwrap());
        CachePolicyBuilder::new(request)
            .build(&response)
            .to_archived()
    }

    /// A server or proxy can send an `Age` header as large as `u64::MAX`.
    /// RFC 9111 S4.2.3 age calculations must not overflow when they include the response's
    /// resident age. Saturating arithmetic limits the age to `Duration::from_secs(u64::MAX)`.
    /// The response must remain stale without a debug panic or release-mode overflow.
    /// This also applies when the freshness lifetime reaches `u64::MAX`.
    #[test]
    fn age_saturates_on_huge_age_header() {
        let request =
            reqwest::Request::new(http::Method::GET, "https://example.com/".parse().unwrap());
        let http_response = http::Response::builder()
            .status(http::StatusCode::OK)
            .header(http::header::AGE, u64::MAX.to_string())
            .header(http::header::CACHE_CONTROL, format!("max-age={}", u64::MAX))
            .body(Vec::new())
            .unwrap();
        let response = reqwest::Response::from(http_response);

        let policy = CachePolicyBuilder::new(&request).build(&response);
        let archived = policy.to_archived();

        // Set `now` after the response timestamp so `resident_age` is nonzero.
        // Adding that value to the initial `u64::MAX` age would otherwise overflow.
        let now = SystemTime::now() + Duration::from_secs(5);
        assert_eq!(archived.age(now), Duration::from_secs(u64::MAX));
        assert!(!archived.is_fresh(now, &request));
    }

    #[test]
    fn stale_request_ignores_freshness_and_vary() {
        let mut original = http_request(http::Method::GET, "https://example.com/");
        original
            .headers_mut()
            .insert(http::header::ACCEPT, "application/json".parse().unwrap());
        let archived = archived_cache_policy(
            &original,
            http::Response::builder()
                .status(http::StatusCode::OK)
                .header(http::header::CACHE_CONTROL, "no-cache")
                .header(http::header::VARY, "accept"),
        );

        let mut request = http_request(http::Method::GET, "https://example.com/");
        request
            .headers_mut()
            .insert(http::header::ACCEPT, "text/html".parse().unwrap());

        assert!(archived.matches_stale_request(&request));
        assert_matches!(
            archived.before_request(&mut request),
            BeforeRequest::Stale(_)
        );

        let request = http_request(http::Method::HEAD, "https://example.com/");
        assert!(archived.matches_stale_request(&request));
    }

    #[test]
    fn stale_request_rejects_non_matching_cache_entries() {
        let original = http_request(http::Method::GET, "https://example.com/");
        let archived = archived_cache_policy(
            &original,
            http::Response::builder().status(http::StatusCode::OK),
        );

        let different_uri = http_request(http::Method::GET, "https://example.com/other");
        assert!(!archived.matches_stale_request(&different_uri));

        let unsupported_method = http_request(http::Method::POST, "https://example.com/");
        assert!(!archived.matches_stale_request(&unsupported_method));

        let unstorable = archived_cache_policy(
            &original,
            http::Response::builder()
                .status(http::StatusCode::OK)
                .header(http::header::CACHE_CONTROL, "no-store"),
        );
        assert!(!unstorable.matches_stale_request(&original));
    }
}
