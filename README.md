# uv 0.9.17: large wheel metadata ZIP-read failure via CloudFront, resolution succeeds via S3 origin

Issue: astral-sh/uv#21668

Classification: bug

Current repository state: the reporter subsequently replaced the live issue with a withdrawal notice and closed it as opened in the wrong repository. The handoff below classifies the report delivered in the issue event.

## Summary

The report describes uv 0.9.17 resolving `studio[ui,gpu]==3.10.0` on Windows for an ARM64 manylinux target through a custom CloudFront-backed package index. For a 1,563,651,157-byte wheel, uv logs `Range requests not supported ...; streaming wheel`, then reports `Failed to read from zip file`, treats the package as invalid, and declares the dependency set unsatisfiable. A comparison using the public S3-origin URL resolves successfully, and a seekable Python reader backed by ranged S3 GET requests can read the central directory and METADATA.

The evidence establishes where uv fails but not why. The CDN check shown in the report is a ranged HEAD request, so it does not establish whether a ranged GET works. The successful comparison also changes more than the artifact URL, and matching multipart ETags do not independently prove identical complete response bodies. CloudFront delivery, uv's range detection, a truncated or timed-out sequential response, and ZIP streaming behavior therefore remain distinguishable possibilities.

Current source follows the reported sequence when an index lacks PEP 658 metadata: try partial remote ZIP reads, mark range requests unsupported when that path is rejected, then scan a sequential GET stream for METADATA. No inspected issue confirms the combined CloudFront, 1.56 GB, sequential-stream ZIP failure.

## Draft response

The log establishes that uv 0.9.17 abandoned ranged metadata access and then failed while reading the sequential wheel stream; it does not establish whether the CDN response, ZIP streaming, or another network error caused that failure. This overlaps astral-sh/uv#11636 and astral-sh/uv#21349, but neither currently covers the combined behavior.

Could you retry a package-only dry run with current uv, a fresh cache, and `-vvv`, and include the complete error chain around the failed wheel? Please also test a ranged GET—not only HEAD—against the CloudFront URL and report the status, `Content-Range`, and `Content-Length`. Those results will show whether this is still reproducible and whether uv is misclassifying range support or failing on the subsequent full stream.

## Classification

This is a bug report because uv 0.9.17 rejects the selected dependency during remote metadata resolution and presents the transport-dependent read failure as an invalid package and unsatisfiable dependency, while the origin path can read the wheel metadata. That is incorrect resolver behavior if it remains reproducible. The fallback and diagnostic requests are possible remedies, but they do not make the reported correctness failure an enhancement or support question.

It is not classified as a duplicate. astral-sh/uv#11636 covers the same final resolver message through non-PyPI proxies, while astral-sh/uv#21349 covers the preceding range-to-stream fallback. Neither establishes the combined trigger or a shared confirmed mechanism. The report also does not establish a regression: historical retry fixes predate uv 0.9.17, and the current behavior has not yet been tested.

## Related

- astral-sh/uv#11636 — Open. This is the closest error-path match: remote metadata resolution through a Nexus proxy intermittently reports an invalid package and `Failed to read from zip file`. A maintainer connected that case to retry handling around the async ZIP reader. Its intermittent, multi-package Nexus behavior differs from the deterministic-looking CDN/large-wheel report, and a later reporter attributed one manifestation to different mirror URLs.
- astral-sh/uv#21349 — Open. This is the closest range-path match: uv consumes full wheel streams on an Azure feed whose range requests work. It does not fail to read the ZIP. The new report has not established that ranged GET works through CloudFront, so it cannot yet be centralized there.
- astral-sh/uv#14395 — Open. This is an adjacent deterministic large-wheel transport case: uv's upstream ZIP reader times out through an alternate mirror while browser, curl, and wget succeed. It uses `--find-links` and exposes a timeout rather than the generic invalid-format resolver result.
- astral-sh/uv#15626 — Merged. This pull request retained the underlying error chain for resolver failures presented as invalid package formats, directly relating to the request for actionable diagnostics. It predates uv 0.9.17, so a complete `-vvv` trace on current uv is needed to determine whether the underlying error is available but absent from the selected excerpt.

## Search and supporting evidence

Searches covered open and closed issues plus open, closed, and merged pull requests. Literal terms included `Failed to read from zip file`, `Range requests not supported`, `streaming wheel`, `UnexpectedEof`, and `upstream reader returned an error`. Conceptual searches covered CloudFront and S3, large wheels, remote wheel metadata, range detection using HEAD versus ranged GET, proxy/CDN transport failures, retry behavior, invalid-package diagnostics, and buffered or full-download fallback. Version-oriented searches examined fixes merged both before and after uv 0.9.17.

The range and redirect chain included astral-sh/uv#18998, astral-sh/uv#21347, astral-sh/uv#21349, astral-sh/uv#2025, astral-sh/uv#3255, astral-sh/uv#2843, astral-sh/uv#3460, and astral-sh/uv#14126. The retry and error-reporting chain included astral-sh/uv#9246, astral-sh/uv#9253, astral-sh/uv#4402, astral-sh/uv#11636, and astral-sh/uv#15626. Streaming and download fallback history included astral-sh/uv#1792, astral-sh/uv#18620, and astral-sh/uv#18688.

Two especially plausible candidates were ruled out. astral-sh/uv#17847 also involved a CloudFront redirect, but its failure was a 404 on a redirected range read and the reporter later confirmed an index-side fix. astral-sh/uv#18688 added a full-download fallback for direct-URL wheel installation failures; this report instead fails while resolving registry wheel metadata, so that merged change does not establish a fix after uv 0.9.17.
