# Check if a site's connection is secure - Computer - Google Chrome Help

Issue: astral-sh/uv#23402

Classification: question

## Summary

The issue contains only a link to Google Chrome Help about checking a site's connection-security status in the browser. It does not describe a warning the reporter saw, identify an affected site, or explain how uv is involved. The public issue matches the event and has no comments with additional details.

## Draft response

The linked article explains how to check a site's connection-security status in Chrome. If you're reporting a uv problem, please share the exact command, your OS, your uv and Python versions, and what you expected versus what happened, including any error output with secrets removed. If you saw a difference between Chrome and uv when connecting to the same server, please include the affected hostname and any relevant proxy or certificate settings, without credentials. If the issue only occurs in Chrome, please use Chrome's support channels.

## Classification

`question` is the best fit because the connection to uv needs clarification. The link alone does not establish a uv bug, enhancement request, or duplicate.

## Reproduction

Outcome: `needs_more_information`.

No uv command or test was run. The report provides no command, configuration, platform, uv or Python version, affected input, or expected versus actual behavior. The article documents Chrome's browser UI, not a uv invocation; testing an arbitrary HTTPS connection would not reproduce a reported symptom. For a uv investigation, request the exact command, OS, uv and Python versions, and expected and actual output with secrets removed. If TLS is involved, also ask for the affected hostname and relevant proxy or certificate settings.

## Related

No meaningfully related issues or pull requests were found. The checkout's TLS certificate documentation (`docs/concepts/authentication/certificates.md`) describes how uv verifies certificates for package indexes and other HTTPS servers. The report does not describe a discrepancy between that behavior and Chrome's site-security indicators.

The strongest adjacent candidates were ruled out: astral-sh/uv#19903 reports a specific uv TLS handshake error even when browsers accept the certificate; astral-sh/uv#9629 concerns how uv presents `UnknownIssuer`; and astral-sh/uv#17427 and its merged pull request astral-sh/uv#18550 concern uv's HTTP client and system verifier. The discussion in astral-sh/uv#16412 and the linked change in astral-sh/uv#16473 concern uv's certificate store configuration. None concern Chrome's site-security indicators, and the new issue reports none of the uv symptoms those discussions cover.
