# How to make simple API responses cachable with etags

Issue: astral-sh/uv#21039

Classification: duplicate

## Summary

The reporter is adding ETag support to devpi-server's Python Simple API responses. With uv 0.12.3, a conditional request receives a 304 response, but uv logs `Server returned unusable 304` and follows it with an unconditional GET. They ask which headers uv needs in order to reuse the cached response.

This is the same HTTP validator-matching behavior already discussed in astral-sh/uv#2253. uv records a strong ETag from the cached response and sends it in `If-None-Match` during revalidation. It accepts the 304 when the response carries the same strong ETag. A matching `Last-Modified` value can also validate the response, but weak ETags are not supported. If none of the stored or returned responses has an ETag or `Last-Modified`, uv can rely on the 304 alone; the warning occurs when validators exist but do not produce a valid match.

The subsequent unconditional GET is deliberate recovery behavior from astral-sh/uv#2218. When a 304 cannot be associated safely with the cached representation, uv fetches a complete response and repairs the cache.

## Draft response

This is the same validator-matching behavior covered by astral-sh/uv#2253. uv stores a strong ETag from the original 200 response and sends it as `If-None-Match` when revalidating. For uv to reuse the cached body, the 304 response should include the same strong ETag value; weak ETags are not supported. A matching `Last-Modified` value can also validate the response.

The second GET without `If-None-Match` is the fallback added in astral-sh/uv#2218: when uv cannot associate the 304 with its cached response safely, it fetches a complete response. Please make devpi return the original strong ETag on both the 200 and 304 responses. If it already does, please share the response headers from the initial 200, the conditional request, and the 304, with authorization and private URL data removed.

## Classification

Classify as `duplicate`. astral-sh/uv#2253 is an open canonical discussion of the same `Server returned unusable 304` warning and fallback request during cache revalidation. Its later repository-implementer report matches this issue especially closely: a server generated an ETag on the original response but omitted it from the 304, and returning that original ETag on the 304 made uv reuse the cached response. The use of devpi-server rather than Nexus or simple-repository-server does not materially change the mechanism or requested guidance.

This is not currently established as a uv bug. Current source implements the validator checks described in RFC 9111 section 4.3.4, and the report does not show response headers demonstrating that the initial and 304 responses contain matching supported validators. It is also not a new enhancement request: the requested ETag revalidation capability already exists.

## Related

### astral-sh/uv#2253 — Nexus doesn't return a `Last-Modified` header (open)

This is the closest and canonical issue. Maintainer analysis states that uv sets `If-None-Match` and `If-Modified-Since`, then checks the 304's validators before reusing the stored response. A later comment reports the identical warning from another Simple API repository implementation. That server returned an ETag on the cached response but omitted it from the 304; adding the original ETag to the 304 removed the warning and extra GET.

### astral-sh/uv#2218 — Fallback to fresh request on non-validating 304 (merged)

This pull request introduced the behavior the reporter observes after the warning. Before the change, a non-validating 304 could flow downstream as though it were a complete response. The merged fallback instead detects an unusable 304 and performs a fresh unconditional request. It explains the second GET but is not an implementation of ETag support prompted by this issue.

## Supporting evidence

Current cache-policy source records response `ETag` and `Last-Modified` values. For stale entries, it adds a cached strong ETag to `If-None-Match`; it deliberately does not use weak ETags. After a 304, it treats the cached response as not modified when either the old and new strong ETags match exactly or the old and new `Last-Modified` timestamps match. If validators are present but do not match, the cached client emits the warning and sends a fresh request.

Searches covered the exact warning, `If-None-Match`, ETag, 304, conditional GET, revalidation, `Last-Modified`, devpi, Simple API caching, and the repository's `cache`, `registry`, and `compatibility` vocabulary across open and closed issues and open, closed, and merged pull requests. Searches for version-specific fixes found no later fix that changes this behavior.

astral-sh/uv#1754 was also inspected through the reference chain but is not listed as a closest related item. It concerns the older downstream symptom `Missing Content-Type` after a 304 was treated as a complete response. astral-sh/uv#2218 fixed that failure by adding the fallback now seen in astral-sh/uv#21039; astral-sh/uv#2253 is the more direct discussion of the validator requirements.
