# Report HTTP proxy errors

Issue: astral-sh/uv#23284

Classification: bug

## Summary

The reporter ran `uv lock --upgrade-package lxml` against a private HTTPS package index in a Debian 13 (trixie) distroless environment with uv 0.13.9 (`x86_64-unknown-linux-musl`) and Python 3.14.9. The error named the index URL, then reported a tunnel connection failure and DNS lookup error without naming the proxy, even with verbose output. The reporter expects the diagnostic to identify the proxy when the proxy connection fails. They report that using the proxy's fully qualified hostname works and that the glibc build installed with pip did not hit the same lookup failure on their setup.

The closest prior work covers request-URL diagnostics, a different HTTPS CONNECT failure, and Docker/musl DNS failures for private repositories. None tracks the missing proxy endpoint in this diagnostic.

## Classification

This is a diagnostic bug: the fetch error shows only the request URL next to a DNS failure that occurred while connecting to the proxy. The missing endpoint is reproducible on both libc builds when the configured proxy cannot be resolved. Whether and why the short proxy hostname fails in the reported distroless environment is a separate question.

## Reproduction

**Outcome: reproducible (missing proxy endpoint).** Tested on Ubuntu 24.04.5 x86-64 with CPython 3.14.10, using uv 0.13.7 (glibc) from `PATH` and the official uv 0.13.9 musl and glibc binaries. A temporary project depended only on `lxml`; a loopback HTTPS simple index served a synthetic `lxml` wheel. A direct run first produced a lockfile. Each subsequent run used a fresh cache and an isolated environment. From the fixture project directory, set `REPRO` to the temporary directory, `INDEX_PORT` to the fixture server's port, and `PYTHON` to the interpreter's absolute path:

```sh
env -i PATH="$PATH" HOME="$REPRO/home" TMPDIR="$REPRO" \
  UV_CACHE_DIR="$REPRO/cache-proxy" UV_HTTP_RETRIES=0 \
  HTTPS_PROXY=http://uv-triage-no-such-proxy.invalid:3128 \
  uv --no-config lock --upgrade-package lxml -v \
  --index-url "https://127.0.0.1:$INDEX_PORT/simple/" \
  --allow-insecure-host 127.0.0.1 --python "$PYTHON" --no-python-downloads
```

- The direct run exited 0, and the localhost index received the package request. With only `HTTP_PROXY` set to the same unresolvable proxy, uv 0.13.9 musl also connected directly and exited 0. For this HTTPS destination the failing run used `HTTPS_PROXY`, not `HTTP_PROXY` alone.
- With `HTTPS_PROXY` set, all three binaries exited 2, and the index received no requests. The error named `https://127.0.0.1:$INDEX_PORT/simple/lxml/` and reported `tunnel error: failed to create underlying connection`, followed by `dns error` and `error resolving DNS`, without naming the proxy. The OS lookup message was `Name does not resolve` on musl and `Name or service not known` on glibc.
- Neither `-v` nor `-vv` exposed the proxy endpoint on the tested musl build. Setting `RUST_LOG=reqwest::connect=debug` in the credential-free fixture did show proxy selection separately; the final error still omitted the endpoint.
- The fixture intentionally used an unresolvable name. It did not reproduce the reporter's single-label versus fully qualified hostname comparison or determine why the short name failed in their container. To investigate that distinction, obtain the effective redacted HTTPS proxy configuration (`HTTPS_PROXY`, `ALL_PROXY`, or `https-proxy`), the image tag or digest, and the container's DNS search configuration.

**Regression tests:** `proxy_https_tunnel_dns_error` and `proxy_http_forward_dns_error` in `crates/uv/tests/it/network.rs` reproduce the omission using an overlong DNS label that glibc rejects locally. The former exercises HTTPS CONNECT setup; the latter demonstrates the same omission for plain HTTP forwarding, where there is no tunnel error to point toward the proxy.

## Fix

**Outcome: fixed.** Both connection paths pass the proxy hostname to the DNS resolver. The default resolver returns the OS lookup error without that hostname, so uv could show the index URL but not say which host failed DNS resolution. The shared client now attaches the attempted hostname at the lookup boundary. It still uses the system resolver, keeps the original OS error and the request URL in the error chain, and does not change proxy selection. Only the hostname is included, not the proxy URL or its userinfo. This fixes the diagnostic, not the container's short-hostname lookup behavior.

The two parent snapshots passed before modification and then failed solely because the new hostname line was absent. With the fix, both pass and distinguish the proxy host from the index URL. Neighboring proxy configuration and routing tests also pass, including a successful CONNECT tunnel, HTTP forwarding, and proxy bypass. The `compile_missing_python_download_error_warning` snapshots in `crates/uv/tests/pip_compile/pip_compile.rs` cover a proxy that is reached but rejects CONNECT, not a DNS lookup failure; no changes were needed there.

Focused debug-profile validation: `cargo test --locked -p uv -p uv-client --test it --no-default-features --features uv/test-python,uv/test-pypi -- network::proxy_ proxy::` passed all nine CLI and two client tests. `cargo fmt --all -- --check` (stable toolchain) and `git diff --check` passed.

## Draft response

We reproduced the missing proxy hostname and updated the diagnostic to show the hostname that failed DNS resolution separately from the index URL. Using the fully qualified proxy hostname remains a workaround if it resolves in your container. For an HTTPS index, check the effective HTTPS proxy configuration; `HTTP_PROXY` alone did not affect the HTTPS request in our test.

## Related

- astral-sh/uv-dev#3135 adds the parent regression tests for DNS failures in HTTPS CONNECT setup and HTTP forwarding.
- astral-sh/uv#8444 (merged) attaches request URLs to network errors, prompted by missing request context in astral-sh/uv#8144. It explains the index URL in the diagnostic but adds no context about the proxy endpoint.
- astral-sh/uv#16360 (closed) includes HTTPS proxy errors during package operations, including `tunnel error: unsuccessful`. That is a different CONNECT failure, not a DNS failure before reaching the proxy. A maintainer pointed to TLS changes in uv 0.11.0 as a possible fix, and the reporter confirmed their setup worked.
- astral-sh/uv#16741 (open) reports DNS failures fetching from a private repository with a Docker/musl build while pip-installed uv works. Its error points to the repository download, with no proxy-tunnel failure or request to report the proxy endpoint.
- astral-sh/uv#8450 (closed) reports the same `Name does not resolve` fragment for private-index access from Docker, without the proxy-tunnel cause. A maintainer closed it after the Docker musl update in astral-sh/uv#16584.
- astral-sh/uv#16584 (merged) moved the Docker builds to bundled musl 1.2.5 to address astral-sh/uv#8450 in November 2025. It changed the build toolchain, not proxy error reporting, and predates the reported release.

## Supporting evidence

- At tag `0.13.9`, the client uses reqwest 0.13.5 and hyper-util 0.1.21. In reqwest's `src/connect.rs`, `connect_via_proxy` sets the tunnel's underlying connection destination to the selected proxy. In hyper-util's `src/client/legacy/connect/proxy/tunnel.rs`, `Tunnel::call` connects to that destination before sending CONNECT and maps a connection failure to `ConnectFailed`. That variant renders as `failed to create underlying connection` and retains the DNS error as its source, but does not render the proxy address.
- In uv's `crates/uv-client/src/error.rs:501` at that tag, the fetch error attaches the request URL. The default `-v` and `-vv` filters in `crates/uv/src/lib.rs:197` and `crates/uv/src/logging.rs:45` do not enable reqwest's debug proxy-selection message; dependency logs are available at higher verbosity or through a targeted `RUST_LOG` filter. This does not fix the final error's missing endpoint.
- When auto-discovering environment proxies, hyper-util's `src/client/proxy/matcher.rs` selects `HTTPS_PROXY` (or its lowercase equivalent) for HTTPS destinations and falls back to `ALL_PROXY`. uv also supports explicit `https-proxy` configuration. The report's `HTTP_PROXY` example alone does not establish which HTTPS proxy configuration was active.

## Search scope

Searched open and closed issues and open, closed-unmerged, and merged pull requests with authenticated GitHub CLI queries. Checked exact tunnel and DNS errors, proxy environment variables, and `uv lock`, then searched separately for proxy selection, verbose logging, endpoint diagnostics, and short-hostname resolution in Docker/musl. Checked the reported release and prior network-error, DNS, and proxy fixes; followed references and maintainer comments on the closest reports. Ruled out astral-sh/uv#6563 (proxy reached; downstream HTTP 404), astral-sh/uv#21781 (host missing from a malformed proxy URL, not DNS failure for a configured host), and astral-sh/uv#20790 (package-index artifact routing, not HTTP transport proxies).

Pull request: https://github.com/astral-sh/uv-dev/pull/3136
