# Report HTTP proxy errors

Issue: astral-sh/uv#23284

Classification: bug

## Summary

On uv 0.13.9 (released November 17, 2026), `uv lock --upgrade-package` fails to fetch an HTTPS package index from a distroless image. The error names the index URL, then reports `tunnel error: failed to create underlying connection` and a DNS lookup failure without identifying the proxy. The reporter also did not see proxy selection in verbose output. They report that using the proxy's fully qualified hostname works and that the glibc build installed with pip does not exhibit the same failure.

The closest prior work covers request-URL diagnostics, a different HTTPS CONNECT failure, and Docker/musl DNS failures for private repositories. None tracks the missing proxy endpoint in this diagnostic.

## Draft response

The DNS failure in this error chain comes from connecting to the proxy, before uv can ask it to open a tunnel to the index. The error only names the index URL, so it points you at the wrong host. The diagnostic should identify the proxy host separately. For now, use the fully qualified proxy hostname as you found. For the HTTPS index in your example, check the effective `HTTPS_PROXY` or `ALL_PROXY` setting if you're configuring the proxy through the environment; `HTTP_PROXY` alone doesn't apply to HTTPS destinations.

## Classification

This is a diagnostic bug: the error correctly reports that the fetch failed but shows only the request URL next to a DNS cause from the proxy connection. The failure stage is established by the source and does not depend on confirming why the short hostname failed on this container. There is no evidence that this diagnostic was previously fixed or that another item tracks it. The report does not establish a general inability for musl to resolve single-label hostnames.

## Related

- astral-sh/uv#8444 (merged) attaches request URLs to network errors, prompted by missing request context in astral-sh/uv#8144. It explains the index URL in the diagnostic but adds no context about the proxy endpoint.
- astral-sh/uv#16360 (closed) includes HTTPS proxy errors during package operations, including `tunnel error: unsuccessful`. That is a different CONNECT failure, not a DNS failure before reaching the proxy. A maintainer pointed to TLS changes in uv 0.11.0 as a possible fix, and the reporter confirmed their setup worked.
- astral-sh/uv#16741 (open) reports DNS failures fetching from a private repository with a Docker/musl build while pip-installed uv works. Its error points to the repository download, with no proxy-tunnel failure or request to report the proxy endpoint.
- astral-sh/uv#8450 (closed) reports the same `Name does not resolve` fragment for private-index access from Docker, without the proxy-tunnel cause. A maintainer closed it after the Docker musl update in astral-sh/uv#16584.
- astral-sh/uv#16584 (merged) moved the Docker builds to bundled musl 1.2.5 to address astral-sh/uv#8450 in November 2025. It changed the build toolchain, not proxy error reporting, and predates the reported release.

## Supporting evidence

- At tag `0.13.9`, the client uses reqwest 0.13.5 and hyper-util 0.1.21. In reqwest's `src/connect.rs`, `connect_via_proxy` sets the tunnel's underlying connection destination to the selected proxy. In hyper-util's `src/client/legacy/connect/proxy/tunnel.rs`, `Tunnel::call` connects to that destination before sending CONNECT and maps a connection failure to `ConnectFailed`. That variant renders as `failed to create underlying connection` and retains the DNS error as its source, but does not render the proxy address.
- In uv's `crates/uv-client/src/error.rs:501` at that tag, the fetch error attaches the request URL. The default `-v` and `-vv` filters in `crates/uv/src/lib.rs:197` and `crates/uv/src/logging.rs:45` do not enable reqwest's debug proxy-selection message; dependency logs are available at higher verbosity or through a targeted `RUST_LOG` filter. This does not fix the final error's missing endpoint.
- When auto-discovering environment proxies, hyper-util's `src/client/proxy/matcher.rs` selects `HTTPS_PROXY` (or its lowercase equivalent) for HTTPS destinations and falls back to `ALL_PROXY`. uv also supports explicit `https-proxy` configuration. The report's `HTTP_PROXY` example alone does not establish which HTTPS proxy configuration was active.

A focused regression test can use an HTTPS request and a proxy that fails to resolve, then check that the error names the proxy endpoint separately from the request target without displaying proxy userinfo. It need not reproduce the container's resolver behavior.

## Search scope

Searched open and closed issues and open, closed-unmerged, and merged pull requests with authenticated GitHub CLI queries. Checked exact tunnel and DNS errors, proxy environment variables, and `uv lock`, then searched separately for proxy selection, verbose logging, endpoint diagnostics, and short-hostname resolution in Docker/musl. Checked the reported release and prior network-error, DNS, and proxy fixes; followed references and maintainer comments on the closest reports. Ruled out astral-sh/uv#6563 (proxy reached; downstream HTTP 404), astral-sh/uv#21781 (host missing from a malformed proxy URL, not DNS failure for a configured host), and astral-sh/uv#20790 (package-index artifact routing, not HTTP transport proxies).
