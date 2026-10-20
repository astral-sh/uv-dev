# `uv self update` fails with SOCKS5 proxy on Windows

Issue: astral-sh/uv#22783

Classification: bug

## Summary

A separate Windows installer bug. astral-sh/uv#10709 and axodotdev/cargo-dist#2078 cover historical HTTP proxy handling; astral-sh/uv#22265 covers NO_PROXY. astral-sh/uv#4227 and astral-sh/uv#7503 added SOCKS support to uv's Rust client, not the PowerShell archive downloader.

On Windows 11 x86_64, the reporter sets `ALL_PROXY=socks5://127.0.0.1:9050` and runs
`uv self update`. With an update available, the installer fails to download
`uv-x86_64-pc-windows-msvc.zip` from both the Astral release mirror and the GitHub
fallback. The error is: “The ServicePointManager does not support proxies with the
socks5 scheme.” The command reportedly exits with status 2.

The reporter tested installed versions 0.11.8 and 0.13.3, both targeting 0.13.4.
An already-current installation skips the failing download. In the same environment,
`uvx uv --version` reportedly downloaded and ran uv 0.13.4 successfully. These are
reporter observations, not independently reproduced results.

## Draft response

uv's Rust HTTP client supports SOCKS5 through astral-sh/uv#7503, but Windows self-update runs a separate PowerShell installer whose archive download uses WebClient/WebProxy. Your ServicePointManager error comes from that download stage. The HTTP proxy fix associated with astral-sh/uv#10709 did not add SOCKS support there.

This needs a SOCKS-capable download path in the cargo-dist PowerShell installer. If your proxy also provides an HTTP endpoint, try setting HTTPS_PROXY to that endpoint before updating; the installer prioritizes it over ALL_PROXY.

## Classification

Windows self-update fails to download an available update through SOCKS5 even though uv's Rust HTTP client supports that proxy scheme. The source confirms a separate PowerShell archive-download path using WebClient/WebProxy, consistent with the reported ServicePointManager error. The closest historical fixes enabled Rust-client SOCKS support or installer HTTP proxy configuration; neither establishes prior SOCKS support in this installer path. No same-problem duplicate or previously fixed regression was established.

This is an update operation failing under an otherwise supported proxy configuration.
The adjacent HTTP proxy authentication and bypass-list issues have different failure
mechanisms and expected fixes. There is no evidence that SOCKS5 archive downloads
previously worked through this Windows installer, so this is not classified as a
regression of the historical Rust-client SOCKS fix.

## Related

- astral-sh/uv#10709 (issue, closed) — The uv installer does not respect HTTPS_PROXY on Windows. The same Windows self-update archive-download stage failed while version checks and other uv downloads worked. Maintainer discussion identified the PowerShell installer. Its HTTP proxy configuration/authentication problem was fixed through axodotdev/cargo-dist#2078; the new report instead rejects the SOCKS5 scheme after reading ALL_PROXY.
- axodotdev/cargo-dist#2078 (pull request, merged) — Powershell installer: respect HTTPS_PROXY and ANY_PROXY env vars. The upstream fix linked from astral-sh/uv#10709 added explicit proxy environment handling to the PowerShell installer. Despite ANY_PROXY in its title, the implementation reads HTTPS_PROXY and ALL_PROXY and constructs System.Net.WebProxy for WebClient. It did not introduce a SOCKS-capable downloader.
- astral-sh/uv#22265 (issue, open) — standalone installer for Windows - NO_PROXY environment variable is not applied. Another current defect in the same generated PowerShell proxy helper. It concerns bypassing proxies for internal hosts, not connecting through SOCKS5. Its upstream issue axodotdev/cargo-dist#2538 and open fix axodotdev/cargo-dist#2539 only address NO_PROXY, so they do not centralize this report.
- astral-sh/uv#4227 (issue, closed) — Support for SOCKS5 and SOCKS5H. The historical SOCKS support request was closed with astral-sh/uv#7503. It concerns uv's reqwest client, whereas this report fails inside the separate PowerShell installer; it is not evidence that Windows self-update previously supported SOCKS5.
- astral-sh/uv#7503 (pull request, merged) — Add `socks` support. Merged on 2024-09-18, this enabled reqwest's socks feature and added its dependency. The diff does not change the PowerShell installer. It supports the distinction between successful uvx downloads and failing self-update archive downloads, rather than establishing a regression.

## Supporting evidence

The checkout examined is commit `5477928f71f0634db593a8a10a8012044fe02349`.

- `Cargo.toml:260` enables reqwest's `socks` feature. The diff for
  astral-sh/uv#7503 confirms that the original support changed the Rust dependency
  configuration, not a Windows installer.
- `crates/uv/src/commands/self_update.rs:177` checks whether an update is needed
  before executing the installer. This matches the report's requirement that an
  actual update be available.
- `crates/uv/src/commands/self_update.rs:337` downloads the target version's
  installer with uv's client and then runs it.
  `crates/uv/src/commands/self_update.rs:516` invokes `powershell` on Windows.
  Successful version resolution or installer-script retrieval therefore does not
  demonstrate that the later archive download supports the same proxy schemes.
- `dist-workspace.toml:9` pins cargo-dist 0.32.0 and selects PowerShell installers.
  The upstream template
  https://github.com/axodotdev/cargo-dist/blob/v0.32.0/cargo-dist/templates/installer/installer.ps1.j2
  constructs `System.Net.WebProxy` at line 242; reads `HTTPS_PROXY`, falling back
  to `ALL_PROXY`, at lines 260–265; and creates `Net.Webclient`, assigns the proxy,
  and invokes its archive download at lines 306–314.
- The discussion on astral-sh/uv#10709 traces the older failure to the installer,
  links axodotdev/cargo-dist#2075 and its replacement axodotdev/cargo-dist#2078,
  and reports release of the fix in cargo-dist 0.30.0. The merged
  astral-sh/uv#15798 adopted cargo-dist 0.30.0 on 2025-09-22. That fix handled
  HTTP proxy environment settings; it did not replace the downloader with a
  SOCKS-capable client.
- astral-sh/uv#22265 links through its timeline to axodotdev/cargo-dist#2538
  and axodotdev/cargo-dist#2539. Their descriptions concern proxy bypass lists
  and retain the WebProxy-based design. They are not fixes for SOCKS5 transport.

The source confirms the separate download paths and the installer's proxy selection.
The exact runtime exception and version matrix come from the report. Retrieval of
the published 0.13.4 installer asset failed during this review, so artifact-level
verification relies on the pinned generator template rather than a downloaded
0.13.4 script. No Windows reproduction or test execution was performed.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues using ServicePointManager, "does not support proxies", SOCKS5, SOCKS5H, socks, ALL_PROXY, "self update" with proxy/SOCKS5/reqwest, installer proxy, PowerShell proxy, WebClient, and area:windows proxy searches, including searches without versions or platforms. Searched open, closed, and merged pull requests for proxy, socks/SOCKS5, self update, and cargo-dist. PR searches returned no matches even for known linked history, so inspected referenced PRs directly, including astral-sh/uv#7503 and astral-sh/uv#15798, and followed upstream discussions and fixes. Inspected candidate comments and the astral-sh/uv#22265 cross-reference chain to axodotdev/cargo-dist#2538 and axodotdev/cargo-dist#2539. Ruled out astral-sh/uv#10709 as the same bug because it addressed HTTP proxy configuration/authentication; astral-sh/uv#22265 addresses NO_PROXY. Also ruled out astral-sh/uv#15970 (mirror configuration), astral-sh/uv#10724 (certificate trust), and astral-sh/uv#16309 (generic WebClient exception without a confirmed SOCKS mechanism). Checked current uv source and the pinned cargo-dist v0.32.0 PowerShell template. No existing item found tracks this exact SOCKS5 archive-download failure.

The report was decomposed before searching into: archive-download failure versus
successful version checks; SOCKS5 support under `ALL_PROXY`; the Windows
standalone self-update subsystem; the available-update trigger and version matrix;
and the exact ServicePointManager error. Searches for observable symptoms were
kept separate from searches for the proposed WebClient/WebProxy mechanism.

Additional reviewed history includes astral-sh/uv#7484, which requested the Rust
client feature implemented by astral-sh/uv#7503, and astral-sh/uv#15696, which
maintainers explicitly redirected to astral-sh/uv#10709. Neither establishes a
duplicate for this installer-specific SOCKS5 failure.

## Next step

Investigate a SOCKS-capable archive download mechanism in the cargo-dist PowerShell
installer, then verify an actual update on Windows with a working SOCKS5 proxy.
A version-check-only test will miss the failure. An HTTP endpoint from the same
proxy service is a conditional workaround because the installer prefers
`HTTPS_PROXY`; this workaround was not independently tested.
