# pip-style extra index url retry limit

Issue: astral-sh/uv#21037

Classification: duplicate

## Summary

The reporter keeps a private package index in user-level pip configuration. The index is reachable
only on the company network or VPN. With pip, they set the retry count to zero so the index can be
used when reachable without incurring a long retry delay when off-site. They request equivalent uv
behavior.

The report combines two behaviors:

- limiting HTTP retries for an unreachable index; and
- allowing resolution to continue through other configured indexes when that index cannot be
  reached.

uv already accepts the global `UV_HTTP_RETRIES` environment variable, with a default of three, and
the network integration tests exercise a value of zero. This can remove retry attempts, but it does
not make the final connection failure non-fatal. The requested optional-index fallback is already
tracked by astral-sh/uv#15803. The broader missing or unavailable index behavior is also tracked by
astral-sh/uv#16301.

## Draft response

uv already supports `UV_HTTP_RETRIES=0` globally, which disables HTTP retries, but that only
shortens the failure path; it does not make an unreachable configured index optional. The request
to ignore an index connection failure and continue with another index is already tracked in
astral-sh/uv#15803, with the broader missing or unavailable index case discussed in
astral-sh/uv#16301. Let's centralize the behavior discussion in astral-sh/uv#15803.

## Classification

This is a duplicate of astral-sh/uv#15803. That open issue has the same triggering condition—a
private index available only under particular network or VPN configurations—and requests the same
outcome: opt in to ignoring the connection error and continue successfully with another index.
The new issue describes pip's zero-retry configuration as the desired interface, but that interface
difference does not change the underlying requested capability.

The issue is not classified as a bug because repository evidence treats failure to query a
configured index as intentional by default. Maintainer discussion in astral-sh/uv#13358 explains
that silently ignoring index failures can have security consequences. The existing canonical
requests therefore ask for an explicit, less strict fallback policy as an enhancement.

## Related issues and pull requests

### astral-sh/uv#15803 — Add config to allow connection errors when searching across indexes

State: open

This is the closest match and the canonical discussion. It describes a private index reachable only
with certain network configuration, specifically including a company VPN, followed by another
usable index. It requests per-index configuration to ignore the connection error so resolution can
still succeed. A maintainer identifies this as potentially resembling a mirror or alternative URL
for the same index, which supplied additional repository vocabulary for the conceptual search.

### astral-sh/uv#16301 — Feature request: Add option to ignore missing/unavailable indexes

State: open

This is the broader version of the same capability. Its original example is a missing local
wheelhouse, but a later comment describes a global private index that works on-site, is unreachable
off-site, and causes `Request failed after 3 retries` delays even for public-only projects. That
comment closely matches astral-sh/uv#21037's trigger and desired fallback.

## Supporting evidence

- `UV_HTTP_RETRIES` is defined as the number of retries for HTTP requests and defaults to three.
- Environment parsing accepts the retry count as an unsigned integer, and the resolved network
  settings pass it to the HTTP client globally.
- The index connection-timeout integration test explicitly sets `UV_HTTP_RETRIES` to zero,
  confirming that zero is supported.
- Current per-index `ignore-error-codes` configuration applies to HTTP status codes; it does not
  provide an equivalent opt-in for connection failures.
- astral-sh/uv#13358 records the maintainer position that a configured index failure should be fatal
  by default because silently continuing can be insecure, while allowing explicit opt-in behavior
  for selected failure cases.

## Search coverage

Open and closed issues and open, closed, and merged pull requests were searched with literal terms
from the report, including `extra-index-url`, `retries`, `UV_HTTP_RETRIES`, `pip.ini`, private index,
timeout, unavailable, unreachable, and VPN. Conceptual searches covered ignoring connection errors,
optional or missing indexes, index fallback and failover, off-site access, mirrors, and alternative
index URLs. Fix-oriented searches covered HTTP retry and timeout changes, nested retry loops, and
404 handling after retries.

The comments and reference chains for the strongest results were inspected. astral-sh/uv#7924 was
plausible because it also involves corporate internal and external index access and a connection
failure, but it requests replacement rather than concatenation of index configuration, not
automatic fallback when an index is unavailable. astral-sh/uv#13985 discusses mirror selection but
requests speed-based selection rather than availability fallback. Merged astral-sh/uv#14996 fixes a
404 being mishandled after a retry, and merged astral-sh/uv#17274 fixes nested retry accounting;
neither implements optional fallback after a connection failure. No pull request was found that
implements the behavior requested here.
