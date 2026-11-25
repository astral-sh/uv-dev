# Check if a site's connection is secure - Computer - Google Chrome Help

Issue: astral-sh/uv#23402

Classification: question

## Summary

The issue contains only a link to Google Chrome Help about checking a site's security status in the browser. No related uv issue or pull request was found. The closest results concern uv's own HTTPS certificate verification and error messages, not Chrome's site-security indicators.

## Draft response

The linked article explains how to check a site's security status in Chrome. If you're reporting a problem with uv, please share your uv version, the command you ran, and what you expected to happen versus what happened. Include any error output with secrets removed. If the issue only occurs in Chrome, please use Chrome's support channels.

## Classification

`question` is the best fit because the connection to uv needs clarification. The issue has no comments and gives no uv command, version, error, claimed incorrect behavior, or feature request. The link alone does not establish a uv bug, enhancement request, or duplicate.

## Related

No meaningfully related issues or pull requests were found.

## Supporting evidence

The checkout's TLS certificate documentation (`docs/concepts/authentication/certificates.md`) describes how uv verifies certificates for package indexes and other HTTPS servers. That is separate from how Chrome displays a site's security status. The report does not describe a discrepancy between the two.

The search covered open and closed issues and open, closed, and merged pull requests. Literal searches used the exact title, “Google Chrome Help,” and the article path and ID. Conceptual searches covered browser connection-security indicators and Chrome's privacy-warning text. Separate searches covered uv certificate verification, certificate diagnostics, system trust, and merged TLS fixes. The browser-focused searches found no other report of the same behavior or request.

The strongest adjacent candidates were ruled out: astral-sh/uv#19903 reports a specific uv TLS handshake error even when browsers accept the certificate; astral-sh/uv#9629 concerns how uv presents `UnknownIssuer`; and astral-sh/uv#17427 and its merged pull request astral-sh/uv#18550 concern uv's HTTP client and system verifier. The discussion in astral-sh/uv#16412 and the linked change in astral-sh/uv#16473 concern uv's certificate store configuration. None concern Chrome's site-security indicators, and the new issue reports none of the uv symptoms those discussions cover.
