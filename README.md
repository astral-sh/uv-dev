# `uv self update` fails with SOCKS5 proxy on Windows

Issue: astral-sh/uv#22783

Classification: bug (reporter reproduced the Windows PowerShell failure; independent Windows verification remains outstanding)

## Summary

On Windows 11 x86_64, the reporter sets `ALL_PROXY=socks5://127.0.0.1:9050`
and runs `uv self update` with an update available. Both the Astral mirror and
GitHub fallback reportedly fail to download the Windows archive with “The
ServicePointManager does not support proxies with the socks5 scheme.” The
reported uv exit status is 2.

The reported version pairs are 0.11.8 → 0.13.4 and 0.13.3 → 0.13.4. The latter
installed version is identified as `uv 0.13.3 (502f93ca6 2026-10-19
x86_64-pc-windows-msvc)`. The reporter says `uvx uv --version` successfully
obtained and ran 0.13.4 in the same environment. No last known-good self-update
version is identified. Python is unspecified and is not required for the
standalone updater. Expected behavior is a successful update through SOCKS5.

The published 0.13.4 PowerShell installer was retrieved and inspected. It has a
separate WebClient/WebProxy archive downloader. However, that downloader's
helpers successfully downloaded the actual Windows archive through a working
SOCKS5 proxy on the available Linux PowerShell 7.6.7/.NET 10.0.13 runtime. SOCKS5 support in this download path therefore depends on the runtime.
The reporter additionally reproduced the same exception using WebClient/WebProxy
under Windows PowerShell 5.1/.NET Framework, while the same download succeeded
under PowerShell 7/.NET 10. This supports a runtime-specific failure and agrees
with the successful Linux control. Windows PowerShell, which uv invokes on
Windows, was unavailable on the investigation runner; independent Windows
verification remains outstanding.

## Classification

Retain the bug classification for the reported failure of a Windows update
under a SOCKS5 configuration. The reporter's isolated comparison now identifies
Windows PowerShell 5.1/.NET Framework as a failing runtime and PowerShell 7/.NET 10
as a successful runtime for the same WebClient/WebProxy download. Together with
the verified invocation of `powershell` by uv, this strengthens the evidence
for a runtime-specific installer networking failure. The comparison is reported
evidence, not an independently executed Windows reproduction. Exact runtime
builds, the standalone download test script, and the executable resolved by the
failing updater are not provided.

No previously working Windows SOCKS5 self-update version or same-problem
duplicate has been established. Historical Rust-client SOCKS support and
installer HTTP proxy fixes concern different paths or proxy behavior. The
failure should not be classified as a demonstrated regression of those fixes.

## Reproduction

**Independent reproduction outcome: `needs_more_information`.** The recorded
Linux controls remain unchanged. The reporter has now reproduced the exception
in a separate Windows PowerShell 5.1 WebClient/WebProxy test and reports success
under PowerShell 7/.NET 10. Independent verification of the Windows downloader
and full update remains outstanding. Success under modern PowerShell does not
justify a `not_reproducible` conclusion for Windows PowerShell.

### Reported minimal scenario

On Windows 11 x86_64, start a working SOCKS5 proxy, install standalone uv 0.13.3,
and run in PowerShell while 0.13.4 is available:

```powershell
$env:ALL_PROXY = "socks5://127.0.0.1:9050"
uv self update 0.13.4
```

The explicit target pins the reporter's upgrade; the reported command omits the
target. An already-current version or `--dry-run` skips the archive download
and cannot reproduce that stage. Record whether `HTTPS_PROXY` is set, without
exposing its credentials: the published installer prefers it over `ALL_PROXY`.
For an isolated test, unset competing proxy variables and use an unauthenticated
loopback proxy. This Windows scenario was not executed on this Linux runner.

### Reporter-provided runtime comparison

In the follow-up on astral-sh/uv#22783 (comment ID 6309873243, 2026-10-21), the
reporter describes a separate download test using `System.Net.WebClient` and
`System.Net.WebProxy` with `ALL_PROXY=socks5://127.0.0.1:9050`:

| Runtime | Reported result |
| --- | --- |
| Windows PowerShell 5.1 / .NET Framework | `System.NotSupportedException: The ServicePointManager does not support proxies with the socks5 scheme.` |
| PowerShell 7 / .NET 10 | The same download succeeds. |

The failure matches the original updater exception and isolates the comparison
to the downloader APIs used by the installer. The comment does not include the
exact script, download URL, or full runtime build numbers. It reports a separate
API-level test, not a new full self-update run or a tested updater workaround.

### Executed controls and observations

- Host: Linux x86_64, kernel `7.0.0-1017-azure`.
- Installed executable: `/opt/hostedtoolcache/uv/0.13.4/x86_64/uv`, reporting
  `uv 0.13.4 (x86_64-unknown-linux-gnu)`.
- Available PowerShell: `/usr/bin/pwsh`, version 7.6.7, .NET 10.0.13.
  Windows `powershell.exe` and a Windows execution environment were unavailable.
- Reproduction files and caches: `/tmp/uv-22783-81fpfK`. Subprocesses used an
  explicit environment with temporary XDG and uv cache directories, no inherited
  proxy settings or tokens, and a locally implemented unauthenticated SOCKS5
  forwarding proxy at `socks5://127.0.0.1:58013`. The proxy was stopped after
  the controls completed.

The harness is `/tmp/uv-22783-81fpfK/probe.py`; running it with `python3` starts
a fresh local proxy and executes these two controls explicitly:

1. `uv self update 0.13.3 --dry-run --no-config`, using the installed uv on PATH
   and a temporary synthetic install receipt supplied through
   `AXOUPDATER_CONFIG_PATH`. The receipt matches the installed binary and has
   `modify_path=false`. This exited 0 and printed:

   ```text
   info: Checking for updates...
   Would update uv from v0.13.4 to v0.13.3
   ```

   The proxy recorded one SOCKS5 connection. This verifies the Rust client's
   version-resolution path through SOCKS5. The explicit older target permits
   exercising version resolution with the installed current uv; this was a
   dry run, not an actual downgrade or the reported Windows upgrade. No
   installed binary was changed.

2. `pwsh -NoLogo -NoProfile -NonInteractive -File download-probe.ps1`.
   The probe extracts the unchanged `WebProxyFromUrl`,
   `WebProxyFromEnvironment`, `Get-ExceptionMessage`, and
   `Invoke-DownloadFile` function definitions from the published 0.13.4
   `uv-installer.ps1`. It creates `Net.Webclient`, assigns the proxy returned
   by the installer helper, and calls:

   ```powershell
   Invoke-DownloadFile -client $client -url 'https://releases.astral.sh/github/uv/releases/download/0.13.4/uv-x86_64-pc-windows-msvc.zip' -path (Join-Path $PSScriptRoot 'uv-windows.zip')
   ```

   This exited 0, printed `Archive download succeeded`, and established one
   SOCKS5 connection. The downloaded ZIP is 15,237,259 bytes, contains
   `uv.exe`, `uvw.exe`, and `uvx.exe`, and passes a ZIP CRC check. Its SHA-256
   is `ccec7508a07d9933d7cb1261880633efcf1f30d6e6eddab21823f1dae6f23225`.
   The installer itself has SHA-256
   `97b4489acafc6a9b2a6529eba6ef46d8d936ed53c0a7acfd9fd5eb4927932144`.
   No installer entry point, archive extraction, or binary installation was
   executed. This is a downloader control on modern .NET, not a Windows
   self-update reproduction.

An initial WebClient probe with no listener on port 9050 produced connection
refused, rather than a scheme-rejection error, on this same runtime. The working
proxy control above eliminates that missing-listener condition. The reported
ServicePointManager error and uv exit status 2 were not observed in either
completed control. Logs and machine-readable results are retained beside the
harness in `uv-self-update.log`, `powershell-download.log`, and `results.json`.

### Existing test coverage

- `crates/uv/tests/it/self_update.rs::check_self_update` uses axoupdater's
  install/update harness under CI and asserts the resulting binary can print
  its version. It does not configure or assert SOCKS5 behavior.
- `crates/uv/tests/it/self_update.rs::test_self_update_uses_custom_path_with_ghe_override`
  and `test_self_update_uses_legacy_path_with_ghe_override` use temporary
  receipts, mock release metadata, and `--dry-run`. They assert the proposed
  version change, without running the archive downloader.
- `crates/uv-client/tests/it/proxy.rs::http_proxy` configures an HTTP mock proxy,
  checks status 200, and asserts one proxy request. `no_proxy` excludes the
  target host and asserts zero proxy requests. Neither tests SOCKS5 or the
  PowerShell installer.
- `crates/uv/tests/it/network.rs::proxy_https_proxy_in_uv_toml` tests package
  installation through an HTTP CONNECT proxy. `proxy_url_without_host` checks
  rejection of malformed `socks5:foo` configuration while offline. Neither
  exercises valid SOCKS5 archive downloads.

No existing test covering the reported Windows SOCKS5 self-update scenario was
found in the searched integration suites. No repository tests were added or
run. The self-update integration module is gated by `self-update`; a future
runtime regression test also needs an appropriate Windows/proxy fixture and
review of applicable `test-*` feature gates.

### Remaining verification

Run the pinned 0.13.3 → 0.13.4 update on Windows 11 x86_64 with a working SOCKS5
proxy, an isolated standalone installation, and competing proxy settings
removed. Capture the version and CLR/runtime of the `powershell.exe` actually
resolved by the updater, as well as the uv exit status and sanitized installer
output. The parent shell may be PowerShell 7 while uv starts Windows PowerShell;
recording only the parent shell's version is insufficient. Independently repeat
the reporter's runtime comparison using the published installer helpers, and
retain the exact script and runtime builds with the results. The reported
PowerShell 7 success does not establish that launching `uv self update` from
PowerShell 7 fixes the issue: uv separately invokes `powershell`.
Python/project configuration is not needed for this check.

## Related

- astral-sh/uv#10709 (issue, closed) — The uv installer does not respect HTTPS_PROXY on Windows. The same Windows self-update archive-download stage failed while version checks and other uv downloads worked. Maintainer discussion identified the PowerShell installer. Its HTTP proxy configuration/authentication problem was fixed through axodotdev/cargo-dist#2078; the new report instead rejects the SOCKS5 scheme after reading ALL_PROXY.
- axodotdev/cargo-dist#2078 (pull request, merged) — Powershell installer: respect HTTPS_PROXY and ANY_PROXY env vars. The upstream fix linked from astral-sh/uv#10709 added explicit proxy environment handling to the PowerShell installer. Despite ANY_PROXY in its title, the implementation reads HTTPS_PROXY and ALL_PROXY and constructs System.Net.WebProxy for WebClient. It did not introduce a SOCKS-capable downloader.
- astral-sh/uv#22265 (issue, open) — standalone installer for Windows - NO_PROXY environment variable is not applied. Another current defect in the same generated PowerShell proxy helper. It concerns bypassing proxies for internal hosts, not connecting through SOCKS5. Its upstream issue axodotdev/cargo-dist#2538 and open fix axodotdev/cargo-dist#2539 only address NO_PROXY, so they do not centralize this report.
- astral-sh/uv#4227 (issue, closed) — Support for SOCKS5 and SOCKS5H. The historical SOCKS support request was closed with astral-sh/uv#7503. It concerns uv's reqwest client, whereas this report fails inside the separate PowerShell installer; it is not evidence that Windows self-update previously supported SOCKS5.
- astral-sh/uv#7503 (pull request, merged) — Add `socks` support. Merged on 2024-09-18, this enabled reqwest's socks feature and added its dependency. The diff does not change the PowerShell installer. It supports the distinction between successful uvx downloads and failing self-update archive downloads, rather than establishing a regression.

## Supporting evidence

The checkout examined is commit `5477928f71f0634db593a8a10a8012044fe02349`.

- `Cargo.toml` enables reqwest's `socks` feature. The historical change in
  astral-sh/uv#7503 concerns the Rust HTTP client.
- `crates/uv/src/commands/self_update.rs` checks whether an update is needed,
  downloads the target installer using uv's client, then executes it. Its
  `execute_official_installer` invokes `powershell` on Windows, not `pwsh`.
- `dist-workspace.toml` pins cargo-dist 0.32.0 and selects PowerShell installers.
  The published 0.13.4 script independently confirms the relevant generated
  implementation: lines 244–279 construct WebProxy and choose `HTTPS_PROXY`
  before `ALL_PROXY`; lines 294–301 wrap `DownloadFile`; lines 320–328 create
  WebClient, assign the proxy, and perform the archive download. Lines 173–181
  catch download errors and proceed to the fallback URL.
- The published script was retrieved from
  `https://releases.astral.sh/github/uv/releases/download/0.13.4/uv-installer.ps1`.
  The downloader control uses this release script's helpers.
- The 0.13.3 and 0.13.4 release notes contain no self-update SOCKS5 fix. Neither
  the report nor the controls establish a last known-good Windows version.

The separate download paths and proxy selection are verified from source and
the release artifact. Independently observed runtime success is limited to the
Linux controls above. The Windows exception, two failing upgrade pairs, and
separate Windows PowerShell 5.1 versus PowerShell 7 download comparison remain
reporter observations.

## Search scope and excluded candidates

The related-issue search covered astral-sh/uv open and closed issues
and pull requests for ServicePointManager, unsupported proxy schemes,
SOCKS5/SOCKS5H, ALL_PROXY, self-update proxy failures, WebClient, PowerShell,
and cargo-dist. It inspected linked upstream history and the
astral-sh/uv#22265 cross-reference chain to axodotdev/cargo-dist#2538 and
axodotdev/cargo-dist#2539. No same-problem duplicate was established.

The reviewed HTTP proxy history includes astral-sh/uv#10709,
axodotdev/cargo-dist#2075, axodotdev/cargo-dist#2078, and
astral-sh/uv#15798, which adopted cargo-dist 0.30.0. This history concerns
HTTP proxy configuration/authentication. astral-sh/uv#22265 concerns proxy
bypass lists. Neither establishes support for the reported SOCKS5 transport
under Windows PowerShell.

Other excluded candidates were astral-sh/uv#15970 (mirror configuration),
astral-sh/uv#10724 (certificate trust), and astral-sh/uv#16309 (generic
WebClient failure without a confirmed SOCKS mechanism). astral-sh/uv#7484
requested the Rust-client feature implemented by astral-sh/uv#7503;
astral-sh/uv#15696 was redirected to astral-sh/uv#10709. These do not establish
an exact duplicate or a previously working Windows SOCKS5 self-update path.

## Next step

Use the reporter's Windows PowerShell 5.1/.NET Framework failure and
PowerShell 7/.NET 10 success to focus investigation on runtime compatibility
of cargo-dist's PowerShell archive downloader. Independently verify the
published helpers and the complete pinned Windows update before selecting a
fix, and ensure regression coverage exercises the failing runtime rather than
only modern PowerShell. An HTTP endpoint supplied by the same proxy service
may be a workaround when configured through `HTTPS_PROXY`, but that workaround
was not independently tested. Running uv from PowerShell 7 alone is not an
established workaround because the updater invokes `powershell` separately.
