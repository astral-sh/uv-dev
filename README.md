# Disposable GitHub Actions cache proxy

An Ubuntu-only experiment for `astral-sh/uv-dev`. A pinned action's `pre` hook
starts a local TLS proxy for the runner-provided GitHub Actions service hosts.
It rejects the legacy cache API and the v2 `CacheService`, forwards other
requests, and blocks direct connections to the original resolved service IPs
except from the proxy's dedicated UID.

This is not a production security boundary. Root can undo the local policy;
unknown service addresses, previously issued blob URLs, container networking,
other cache backends, and non-Linux runners are outside this prototype. The
proxy handles sensitive runtime authorization in memory and never logs request
headers, bodies, or signed URLs. Its generated CA key is deleted after signing
the narrowly scoped leaf certificate. No real release or cloud credentials are
used; the OIDC test requests a token for a test-only audience and does not
exchange it for cloud credentials.

Run the local synthetic TLS tests with `python3 -m unittest discover -s tests`.
The isolated workflow uses harmless text, exact run-specific cache keys, and
same-run artifacts. It deliberately uses `ACTIONS_CACHE_MODE: write` to prove
that the proxy, rather than the newer client opt-out, blocks cache operations.
