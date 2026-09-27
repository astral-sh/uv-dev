# Can't install packages: UnknownIssuer

Issue: astral-sh/uv#22011

Classification: bug

## Summary

On Windows 11, uv 0.12.19 fails to fetch the PyPI page for `isodate` with `invalid peer certificate: UnknownIssuer`. The failure occurs with both `uv pip install` and `uv add`, and it persists when `--system-certs` or its deprecated alias `--native-tls` is enabled. Exporting the GlobalSign certificates from the current user's Windows trusted-root store, converting them to PEM, and passing that bundle through `uv pip install --cert` succeeds.

The important behavior is independent of PyCharm and of the `isodate` package: a manual uv invocation cannot validate the PyPI certificate through Windows system trust, while the same certificates work when supplied as a custom bundle. PyCharm's inability to pass extra uv arguments is context, not a necessary trigger.

The closest repository history is the Windows native/system-certificate failure tracked in astral-sh/uv#9243 and astral-sh/uv#17355. Reporters confirmed that uv 0.11.0 fixed those failures after the TLS-stack upgrade in astral-sh/uv#18550. Seeing the same observable failure on uv 0.12.19 makes this a possible regression. The exact mechanism is not yet confirmed: a non-empty `SSL_CERT_FILE` or `SSL_CERT_DIR` would override `--system-certs`, and the certificate chain presented on the reporter's network still needs to be compared with the Windows store.

## Draft response

Thanks for the detailed command comparisons. `--system-certs` is expected to delegate certificate verification to Windows. astral-sh/uv#18550 introduced that path and resolved the same `UnknownIssuer` behavior reported in astral-sh/uv#9243 and astral-sh/uv#17355 for uv 0.11.0. Since this occurs on uv 0.12.19 while a PEM exported from the Windows trust store works, this may be a regression.

Could you first confirm that neither `SSL_CERT_FILE` nor `SSL_CERT_DIR` is set in the shell or PyCharm environment? A non-empty value overrides `--system-certs`. If both are unset and the command still fails, please share the output of `uv -vv --system-certs pip install isodate` and `openssl s_client -showcerts -connect pypi.org:443`, with any private proxy details redacted. That will let us compare the certificate chain being served without requiring the exported certificate bundle.

## Classification

This is a bug because uv documents `--system-certs` as using the platform's native certificate store and delegates verification to the operating system through `rustls-platform-verifier`. The report demonstrates that certificates present in the Windows trusted-root store are not accepted through that path, even though uv accepts those certificates after they are exported and supplied as a bundle.

The earlier reports are closed and were confirmed fixed in uv 0.11.0. The current report is against uv 0.12.19, so it should not be classified as a duplicate of those historical reports. It is a possible regression even though the precise cause remains to be isolated. Environment-variable overrides and the served chain are diagnostics to check; neither is established by the report.

## Related

- astral-sh/uv#9243 — Closed. This is the canonical historical discussion. It accumulated Windows and macOS reports in which native/system trust rejected certificate chains accepted by platform tools. Its final confirmation states that uv 0.11.0 fixed the problem.
- astral-sh/uv#17355 — Closed. This is the closest reproduction: on Windows, fetching from PyPI failed with `UnknownIssuer` even though a company root was trusted and native TLS was enabled. The reporter also confirmed that uv 0.11.0 fixed it.
- astral-sh/uv#18550 — Merged. This TLS-stack upgrade shipped for uv 0.11.0, replaced `rustls-native-certs` verification with `rustls-platform-verifier`, and documented that Windows validation would delegate to `CertGetCertificateChain` and `CertVerifyCertificateChainPolicy`.

## Search evidence

The search covered open and closed issues and open, closed, and merged pull requests. Literal queries included `UnknownIssuer`, `invalid peer certificate`, `system-certs`, `native-tls`, `GlobalSign`, and PyCharm. Conceptual queries covered Windows trusted roots and certificate stores, corporate CAs and TLS inspection, certificate-chain validation, `rustls-native-certs`, and `rustls-platform-verifier`. Fix-oriented queries covered reqwest 0.13, the uv 0.11.0 TLS change, and later certificate-handling fixes. Candidate comments and their linked discussions were inspected through the chain from astral-sh/uv#17355 to astral-sh/uv#9243, astral-sh/uv#17427, and astral-sh/uv#18550.

Two plausible open candidates were ruled out as canonical matches. astral-sh/uv#16474 became a custom-PEM validation report after its reporter found that clearing `SSL_CERT_FILE` made native system trust work; in astral-sh/uv#22011, the explicit PEM is the working path. astral-sh/uv#19903 centers on `UnsupportedCriticalExtension` for an enterprise intermediate on Linux, whereas astral-sh/uv#22011 reports `UnknownIssuer` on Windows and succeeds when the certificates are supplied explicitly. The withdrawn configuration report astral-sh/uv#19876 and the environment-conflict resolution in astral-sh/uv#6649 are useful diagnostics but do not track the same confirmed behavior.
