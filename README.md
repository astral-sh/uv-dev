# uv resorts to downloading full wheels even though range requests work

Issue: astral-sh/uv#21349

Classification: duplicate

## Summary

On Ubuntu 24.04 with uv 0.12.7 and Python 3.14, the reporter sees dependency
resolution against an Azure Artifacts feed transfer wheel contents instead of
using HTTP byte ranges to read wheel metadata. The feed does not publish PEP
658 metadata sidecars, but it does support range requests. The reporter points
to the failing integration test in astral-sh/uv#21347 as an accurate model of
the observed behavior.

The closest existing tracker is astral-sh/uv#11379. That open issue covers the
same Azure Artifacts setup: no PEP 658 metadata, ranged GET requests that work
when redirects are followed, uv reporting range requests as unsupported, and
fallback to streamed wheel metadata. Later evidence in that thread specifically
shows uv disallowing the redirect to the final blob host after Azure added HEAD
support. The new report adds a current-version observation and a focused test,
but does not establish a separate problem.

The repository evidence confirms the generic redirect failure in
astral-sh/uv#21347's test. It does not yet confirm that every detail of the
proposed fix is correct for the reporter's live feed. Redirect handling must
also preserve the method-specific signed-URL behavior fixed by
astral-sh/uv#3460.

A uv maintainer has now independently reproduced the range-request issue
against the authenticated Azure feed exercised by the repository's package
registry CI. This confirms that the failure is present in uv's Azure
integration and is not limited to the reporter's environment. The maintainer
did not report additional protocol traces or confirm the proposed root cause,
so the precise redirect/authentication mechanism and fix remain under review.

## Public reproduction target

A commenter provided `https://packagefeedproxy.microsoft.io/pypi/simple` as a
public feed that maintainers can use to investigate without private Azure
Artifacts credentials. The commenter reports that this feed does not publish
PEP 658 metadata and should support range requests. Those capabilities have
not yet been independently verified in the issue, so the feed is a candidate
reproduction target rather than a confirmed reproduction.

Testing should establish the redirect chain and HEAD/ranged-GET responses,
then check whether a cache-disabled resolution on uv 0.12.7 emits the
unsupported-range warning or streams wheel metadata. Any trace retained in
the handoff should remove signed URL parameters even though the starting feed
is public.

## Draft response

Thanks for the report. This is already tracked in astral-sh/uv#11379,
including the Azure Artifacts redirect path where PEP 658 is unavailable,
ranged GETs work, and uv falls back to streaming wheel metadata. The focused
reproduction in astral-sh/uv#21347 is useful additional evidence.

Let's centralize the investigation in astral-sh/uv#11379. If you have
sanitized uv 0.12.7 trace logs, please add them there with credentials and
signed URL parameters removed.

## Classification

This is a duplicate of astral-sh/uv#11379. That issue remains open and tracks
the same subsystem, service, triggering conditions, warning/fallback, expected
range behavior, and redirect dependency. Under the repository's triage rules,
the existing open tracker takes precedence even though astral-sh/uv#21349 adds
a more specific integration reproduction.

The underlying behavior is a bug: when PEP 658 metadata is unavailable and a
feed supports byte ranges after redirect, uv should not misclassify the feed
and take the streamed fallback. A maintainer's analysis on
astral-sh/uv#21347 says the manual redirect wrapper introduced by
astral-sh/uv#14126 is lost when constructing the client used by the range
reader, and identifies that as the apparent regression. This is strong
source-informed evidence, but remains a diagnosis under review rather than a
confirmed live-feed root cause.

"Downloading full wheels" also has an implementation nuance. Since
astral-sh/uv#1792, the fallback streams the archive only until it finds the
METADATA entry, rather than necessarily downloading the complete wheel to
disk. Depending on archive layout it can still consume most of a wheel, so
this nuance does not make the reported excessive transfer correct.

## Related

- astral-sh/uv#11379 — Open canonical tracker for slow Azure DevOps/Azure
  Artifacts resolution. It records the same lack of PEP 658 metadata, working
  range requests after redirects, uv's unsupported-range warning, and streamed
  fallback. Its later comments show the redirect to the blob endpoint being
  disallowed after HEAD support was added.
- astral-sh/uv#21347 — Open proposed fix from the reporter. Its integration
  test demonstrates that HEAD follows a redirect and discovers
  `Accept-Ranges: bytes`, while subsequent range GETs use a client that does
  not follow the redirect and metadata retrieval falls back. Maintainers have
  not accepted the proposed response-URL fix because other registries require
  different handling.
- astral-sh/uv#18998 — Open adjacent GAR report with the same user-visible
  fallback and no PEP 658 metadata. It is not the canonical duplicate because
  there HEAD does not provide a usable range signal; astral-sh/uv#21347 models
  a successful redirected HEAD followed by range GET redirect failure.
- astral-sh/uv#14126 — Merged redirect/authentication change that introduced
  uv's manual redirect wrapper. Maintainer analysis on astral-sh/uv#21347 says
  the range reader does not retain that wrapper and identifies this as the
  apparent regression path.
- astral-sh/uv#3460 — Merged historical fix that deliberately retained the
  originally requested URL for range GETs because some indexes issue different
  presigned URLs for HEAD and GET. This is the principal constraint against
  simply reusing every HEAD response URL.
- astral-sh/uv#1792 — Merged streamed-metadata fallback. It establishes that a
  failed range read does not necessarily save the entire wheel to disk, though
  it may transfer most of the archive before reaching METADATA.

## Search coverage

Literal searches covered `range requests`, `range request`, `PEP 658`, `full
wheel`, `whole wheels`, `Accept-Ranges`, `Content-Range`, `Azure Artifacts`,
and the warning/fallback vocabulary. Conceptual searches covered lazy wheels,
remote wheel metadata, streamed metadata fallback, redirect following,
authentication middleware, index capability caching, and method-specific
presigned URLs. Searches included open and closed issues and open, closed, and
merged pull requests, with direct inspection of comments and referenced
history.

Fix-oriented inspection followed astral-sh/uv#21347 through
astral-sh/uv#14126, astral-sh/uv#7226, astral-sh/uv#3460,
astral-sh/uv#2843, and astral-sh/uv#1792, plus the issues they close or discuss.
astral-sh/uv#18998 was the most plausible alternative canonical issue, but its
failure occurs at HEAD-based capability detection rather than after a
successful redirected HEAD. astral-sh/uv#5073 was also inspected and ruled out
as the canonical tracker because it addresses generic performance of the
streaming fallback, not redirect handling on a range-capable feed.
