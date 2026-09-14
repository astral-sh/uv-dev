# uv seems to fail unpacking a source distribution's tar file

Issue: astral-sh/uv#21641

Classification: duplicate

## Summary

On Linux with uv 0.12.13 and CPython 3.13.6, `uv add wxpython` fails while downloading and building the PyPI source distribution for both wxPython 4.3.1 and 4.2.5. The reported archive member changes between attempts. Installing the same releases with pip works, and `uv add` succeeds when given the already-downloaded local tarball.

The terminal error chain is `error decoding response body` -> `request or response body error` -> `error reading a body from connection` -> `connection reset`. The verbose log records three transient-failure retries followed by a fourth GET, so this is not evidence that the published tarball is malformed and is not the historical failure to recognize a stream error as retryable. The response is being interrupted while uv streams the sdist directly into its extractor; all full-download retries are exhausted.

This is the same user-visible behavior already tracked by open issue astral-sh/uv#13717: a source distribution fails at an archive member during streamed extraction because the HTTP response body ends with a transport error. The low-level transport message differs (`broken pipe` there, `connection reset` here), but the command path, artifact type, extraction stage, error chain, and retry/robustness problem match.

As of 2026-09-14, a maintainer was unable to reproduce the wxPython failure and asked whether antivirus software could be involved. No details about the maintainer's test environment were provided, and antivirus interference remains an unconfirmed hypothesis. Establishing whether the reporter has antivirus, endpoint-security, proxy, or other network-inspection software—and whether the behavior changes when such software is safely bypassed—would help distinguish an environment-specific connection reset from a generally reproducible PyPI download failure.

## Draft response

This is the same streamed source-distribution download failure tracked in astral-sh/uv#13717. The final causes show that the PyPI response is being interrupted by a connection reset while uv is unpacking it; the changing member path and successful install from the local tarball indicate that the tar file itself is not the problem.

Your verbose log also shows that uv retries the complete download three times before failing. astral-sh/uv#21570 added partial-download resumption for wheel downloads, but source distributions are still unpacked as they stream, so that change does not cover this path. We'll keep the source-distribution case centralized in astral-sh/uv#13717; your wxPython reproduction provides an additional user-facing example of it.

## Classification

`duplicate` takes precedence because astral-sh/uv#13717 is open and tracks the same underlying source-distribution stream interruption. A broken pipe and a connection reset are different transport errors, but both surface here only after the response body fails during streamed tar extraction. The varying failed member is a consequence of where each response stops, rather than a distinct archive-format failure.

This is not classified as a regression of astral-sh/uv#14171. That issue concerned managed Python installation downloads, and its fixes in astral-sh/uv#15567 and astral-sh/uv#15675 were scoped to Python and binary streaming downloads. It is also not a recurrence of the missing broken-pipe retry fixed by astral-sh/uv#13281: astral-sh/uv#21641's log explicitly shows three retry delays and four fresh requests.

## Related

- astral-sh/uv#13717 — Open canonical match. Its ecosystem test fails while streaming a registry sdist into `sdists-v9`, with `Failed to extract archive`, member-specific `failed to unpack`, response-body errors, and a terminal broken pipe. The new report has the same behavior with a terminal connection reset.
- astral-sh/uv#12359 — Closed historical user-facing match involving large PySpark sdists. It contains the same `Failed to download and build` and streamed member-unpack error chain ending in a broken pipe. It was closed after the reporter mitigated the CI frequency, not because source-distribution resumption was implemented.
- astral-sh/uv#13281 — Merged retry fix prompted by astral-sh/uv#12359. It teaches streaming downloads to classify broken-pipe failures as retryable. The new log demonstrates that the retry loop runs, then exhausts its budget, so this fix is relevant history but does not resolve repeated interruptions.
- astral-sh/uv#14171 — Closed adjacent report with an almost identical extraction and connection-reset chain during `uv python install`. Maintainer discussion and the closing changes limit that issue to Python and binary downloads, so it is not the canonical package-sdist tracker.
- astral-sh/uv#16934 — Closed request for resumable downloads. It describes uv restarting interrupted package downloads while pip can resume them, which matches the robustness gap exposed here, but it did not specifically track streamed sdist extraction.
- astral-sh/uv#21570 — Merged implementation for astral-sh/uv#16934 that resumes interrupted wheel downloads with HTTP range requests. Its wheel-only scope is the important difference: registry sdists still stream directly into the extractor.

## Supporting evidence

### Report decomposition

- Command and subsystem: `uv add` resolving a registry package, downloading a `.tar.gz` source distribution, extracting it into the `sdists-v9` cache, and building it.
- Triggering conditions: wxPython 4.3.1 or 4.2.5 on Linux; the registry download is interrupted by repeated connection resets. The specific failed archive member varies.
- Expected behavior: the registry sdist downloads, extracts, builds, and installs as it does through pip or after being downloaded locally.
- Actual behavior: four fresh GET attempts fail during streamed extraction, and the final user-facing chain begins with `Invalid tar file` but ends with `connection reset`.
- Exact identifiers and fragments searched: `wxpython`, `Invalid tar file`, `failed to unpack`, `error decoding response body`, `error reading a body from connection`, `connection reset`, `broken pipe`, `sdist`, `source distribution`, streamed unpacking, retries, interrupted downloads, resumption, partial downloads, and HTTP range requests.

### Repository and history evidence

The current source-distribution path in `SourceDistributionBuilder::download_archive` converts `response.bytes_stream()` directly into the reader passed to `ValidatedSourceArchive::extract`. The callback is wrapped by the cached client's retry mechanism, which explains the complete re-downloads in the verbose log. By contrast, the range-resumption implementation from astral-sh/uv#21570 is in `DistributionDatabase::download_wheel_response` and explicitly operates on wheel downloads.

Literal searches across open and closed issues and open, closed, and merged pull requests found astral-sh/uv#13717, astral-sh/uv#12359, and astral-sh/uv#14171 from the exact error fragments. Conceptual and fix-oriented searches covered large source distributions, streamed archive extraction, transient body failures, retry exhaustion, partial downloads, and HTTP range resumption, leading to astral-sh/uv#13281, astral-sh/uv#16934, and astral-sh/uv#21570.

The reporter-suggested astral-sh/uv#14171 was inspected but ruled out as the canonical tracker because it is specific to managed Python downloads. The recent astral-sh/uv#21570 was also ruled out as a fix for this report because its partial-resumption path is wheel-specific. Open astral-sh/uv#15896 shares response-body wording but concerns an Artifactory simple-index TLS/server behavior, not sdist extraction; astral-sh/uv#2138 concerns Azure Artifacts index behavior and is not a close match.

### Follow-up investigation status

A maintainer reported that they could not reproduce astral-sh/uv#21641 and asked about antivirus software. This narrows neither the affected environment nor the mechanism by itself: there is not yet a maintainer environment description, a reporter answer, or an antivirus-enabled/disabled comparison. Treat antivirus or endpoint-security interference as a diagnostic lead, not an established cause.
