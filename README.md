# standalone installer for Windows - NO_PROXY environment variable is not applied

Issue: astral-sh/uv#22265

Classification: duplicate

## Summary

On Windows 11 x64 with uv 0.12.23, the standalone PowerShell installer honors `HTTPS_PROXY` but ignores `NO_PROXY`. The reporter uses `UV_INSTALLER_GITHUB_BASE_URL` to select an internal mirror that is unreachable through the corporate proxy. They also ask where the installer source lives and offer to contribute. No exact runtime error or prior working version is supplied.

Confirmed installer bug, now tracked upstream in axodotdev/cargo-dist#2538 with an open proposed fix in axodotdev/cargo-dist#2539. Earlier Windows proxy fixes addressed different behavior.

## Draft response

The 0.12.23 installer reads HTTPS_PROXY and ALL_PROXY but never NO_PROXY, so it sends internal-mirror downloads through the configured proxy. The installer source is cargo-dist's PowerShell template: https://github.com/axodotdev/cargo-dist/blob/main/cargo-dist/templates/installer/installer.ps1.j2.

Your report is now tracked upstream in axodotdev/cargo-dist#2538, with a proposed fix in axodotdev/cargo-dist#2539. That PR is still open; a fix will need to reach uv's generated installers before it is available in a release. Please coordinate your contribution or validation of your mirror setup on that PR.

## Classification

Open upstream issue axodotdev/cargo-dist#2538 explicitly tracks the same missing NO_PROXY handling in the same PowerShell template, so discussion can be centralized there. This classification rests on that matching issue, not merely on axodotdev/cargo-dist#2539, which was created in response to this report. Source inspection confirms an underlying bug; historical proxy support explicitly omitted NO_PROXY, so there is no evidence of a previously fixed behavior regressing.

The request for contribution guidance does not turn this source-confirmed correctness problem into a question or enhancement. The proposed raw `-BypassList` forwarding in the report should not be treated as an accepted patch; the upstream proposal handles conversion to `WebProxy` bypass patterns explicitly.

## Related

- axodotdev/cargo-dist#2538 — Powershell installer ignores NO_PROXY (issue, open). Tracks the exact PowerShell installer bypass failure, including internal mirrors with HTTPS_PROXY and NO_PROXY set. Opened on October 9 following astral-sh/uv#22265, it provides an open upstream discussion for the same unresolved problem.
- axodotdev/cargo-dist#2539 — Windows installer: respect NO_PROXY env vars (pull request, open). Proposes the exact fix in cargo-dist's PowerShell template and explicitly references astral-sh/uv#22265. Converts NO_PROXY entries into bypass patterns and reports Windows PowerShell 5.1 validation. It remains unmerged and was created in response to this report.
- axodotdev/cargo-dist#2078 — Powershell installer: respect HTTPS_PROXY and ANY_PROXY env vars (pull request, merged). Introduced the installer's HTTPS_PROXY/ALL_PROXY handling in September 2025. Its description explicitly excludes NO_PROXY, establishing that the reported gap was not fixed by the original proxy-support change.
- astral-sh/uv#10709 — The uv installer does not respect HTTPS_PROXY on Windows (issue, closed). Concerns the same generated Windows installer and identifies Net.WebClient as its download path. Resolved through axodotdev/cargo-dist#2078, but concerned ignoring the proxy configuration itself, whereas this report concerns failing to bypass an otherwise working proxy.
- astral-sh/uv#11640 — uv ignore proxy exceptions on Windows (issue, closed). Similar internal-host bypass symptom, but concerned Windows registry proxy exceptions in uv's reqwest package client. Its upstream reqwest fix does not affect the PowerShell installer, so it is neither a duplicate nor evidence of a regression here.

## Search coverage

Used authenticated gh to search open/closed uv issues and open/closed/merged PRs for NO_PROXY, WebProxyFromEnvironment, UV_INSTALLER_GITHUB_BASE_URL, HTTPS_PROXY, installer proxy, proxy exceptions, proxy bypass, PowerShell, and installer source/cargo-dist. Broadened beyond Windows and version identifiers, used area:network terminology, and traced historical fixes and maintainer comments through astral-sh/uv#10709, astral-sh/uv#15798, axodotdev/cargo-dist#2075, and axodotdev/cargo-dist#2078. Upstream searches and recent PR listings found axodotdev/cargo-dist#2538 and axodotdev/cargo-dist#2539. Ruled out astral-sh/uv#11640, astral-sh/uv#2021, astral-sh/uv#14104, and astral-sh/uv#15785 as package-client cases; astral-sh/uv#22267 and astral-sh/uv#9134 concern mirror selection/layout. REST searches became rate-limited, and GraphQL PR searches failed to surface known PRs; direct PR reads and recent listings supplemented them, so exhaustive PR coverage could not be established.

## Supporting evidence

- The [uv 0.12.23 release installer](https://github.com/astral-sh/uv/releases/download/0.12.23/uv-installer.ps1) contains no `NO_PROXY` reference. `WebProxyFromEnvironment` at lines 274–280 reads `HTTPS_PROXY` and `ALL_PROXY` and constructs a proxy without bypass entries. Lines 320–324 assign it to `Net.WebClient.Proxy`.
- The [current cargo-dist template](https://github.com/axodotdev/cargo-dist/blob/main/cargo-dist/templates/installer/installer.ps1.j2#L260) has the same implementation and likewise contains no `NO_PROXY` reference. This independently confirms the code excerpt in the report.
- The checkout configures cargo-dist 0.32.0 in `dist-workspace.toml:9`; `.github/workflows/release.yml:229` invokes `dist build --artifacts=global` to generate global release artifacts. The PowerShell installer template is maintained in cargo-dist.
- axodotdev/cargo-dist#2078 merged September 7, 2025, and explicitly stated that `NO_PROXY` was unsupported. Comments in astral-sh/uv#10709 identify cargo-dist 0.30.0 as the original proxy-support release. astral-sh/uv#15798 merged September 22, 2025, and tracks adoption of that upstream cargo-dist release line.
- astral-sh/uv#22265 was opened October 7, 2026. axodotdev/cargo-dist#2538 and axodotdev/cargo-dist#2539 were opened October 9 and explicitly cite this report. Both remain open. The proposed fix adds bypass matching, including protection against lookalike hostname suffixes; its author reports 13 passing assertions under Windows PowerShell 5.1. These are author-reported results, not independently executed tests.

## Additional candidates ruled out

astral-sh/uv#15696 was closed as a duplicate of astral-sh/uv#10709 and concerned missing `HTTPS_PROXY` handling. astral-sh/uv#2021, astral-sh/uv#14104, and astral-sh/uv#15785 concern `NO_PROXY` behavior or documentation for the reqwest package client. They do not establish installer bypass support.

astral-sh/uv#22267 is the same reporter's separate request concerning `UV_ASTRAL_MIRROR_URL` selection. astral-sh/uv#9134 concerns mirror layout and updater API URLs; astral-sh/uv#15970 asks how to configure updater access. Those questions do not explain why an already selected internal mirror is incorrectly sent through a proxy.

## Maintainer next step

Track and review axodotdev/cargo-dist#2539, and direct contribution or environment-specific validation there. Once a fix is accepted, it must be incorporated into uv's installer-generation toolchain and published installers. No release containing this proposed fix was established.

## Verification limits

The finding is supported by inspection of the released script and upstream template; no Windows runtime reproduction was performed. No checkout files or GitHub resources were modified. The only authored handoff is this README.
