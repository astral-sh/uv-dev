# Fix broken .netrc documentation link

Issue: astral-sh/uv#22513

Classification: duplicate

## Summary

The reporter followed the `.netrc` link in the HTTP authentication documentation's “netrc files” section and found a broken destination. They suggest adding `.html` to the URL. The checkout contains the reported extensionless URL, which currently returns HTTP 404; the suggested destination returns HTTP 200 and has the title “.netrc - everything curl”.

astral-sh/uv#5605 already contains an exact report of this broken .netrc link and the missing .html suffix.

## Draft response

You're right: the documented URL returns 404, and https://everything.curl.dev/usingcurl/netrc.html serves the intended .netrc documentation. This exact broken link was already reported in the September 1 comment on astral-sh/uv#5605. Let's track the correction there; the change needed is to update the URL in docs/concepts/authentication/http.md.

## Classification

The broken link is confirmed in docs/concepts/authentication/http.md:22: its destination returns HTTP 404, while the suggested .html URL returns HTTP 200 and serves the .netrc documentation. However, the identical defect was already reported in the still-open astral-sh/uv#5605 before this issue, so duplicate takes precedence over bug. No evidence of a previously fixed link regression was found.

The report establishes a documentation correctness defect despite the reporter's uncertainty about the intended URL. The duplicate classification rests on a specific earlier comment describing the identical defect, rather than merely on the broad scope of the documentation tracker. The current issue was opened on October 13, 2026, after that September 1 report.

## Related

- astral-sh/uv#5605 — Documentation feedback (open). The September 1, 2026 comment by hendev1 reports the exact same broken URL in docs/concepts/authentication/http.md and identifies the missing .html suffix. This is a specific prior report of the same defect within the open documentation tracker.

## Supporting evidence

- `docs/concepts/authentication/http.md:20` introduces the “netrc files” section. Line 22 links the `.netrc` text to https://everything.curl.dev/usingcurl/netrc.
- A GET request following redirects to https://everything.curl.dev/usingcurl/netrc returned HTTP 404. The effective URL remained unchanged.
- A GET request following redirects to https://everything.curl.dev/usingcurl/netrc.html returned HTTP 200. The effective URL remained unchanged, and the HTML title identifies the intended .netrc documentation.
- The September 1, 2026 comment by hendev1 in astral-sh/uv#5605 (comment ID 5491870196) explicitly identifies `docs/concepts/authentication/http.md`, the same extensionless URL, and the missing `.html` suffix. The tracker remains open; surrounding comments do not identify a fix for this report.
- Closed astral-sh/uv#8482, “Improving netrc documentation,” asked which file locations uv checks. A maintainer explained the `NETRC` environment variable and `~/.netrc` fallback. Merged astral-sh/uv#8511, “Document `.netrc` environment variable and path,” closed that issue on October 24, 2024. Its diff retains the extensionless curl URL, so it is not a historical fix for this defect.

## Search scope

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests using netrc, the exact everything.curl.dev/usingcurl/netrc URL, everything.curl.dev, netrc link, broken/dead documentation links, HTTP authentication, and curl documentation, including the documentation label. Inspected the matching discussion and surrounding comments in astral-sh/uv#5605. Followed closed astral-sh/uv#8482 to merged astral-sh/uv#8511 and inspected its diff; ruled them out as duplicates or prior link fixes because they clarified NETRC and ~/.netrc discovery while retaining the extensionless URL. No matching link-fix pull request was found.

The report has one claim: following the external link from the netrc documentation fails, and the expected behavior is to reach the format reference. No uv command failure, package, version, or platform condition is reported. Literal URL searches and conceptual broken-link searches were kept separate from the proposed missing-suffix explanation. Pull request searches included explicit open, closed, and merged states; the historical documentation pull request was also inspected directly through the issue's timeline.

## Recommended next step

Centralize discussion in astral-sh/uv#5605 and update the external URL in `docs/concepts/authentication/http.md` to the verified `.html` destination. The draft above is for maintainer review.

Only this temporary handoff README was updated. No checkout files or GitHub records were changed. Verification used read-only source inspection and HTTP requests; no build or test run was needed.
