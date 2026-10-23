# Windows Smart App Control blocks unsigned uv-managed CPython and venv executables

Issue: astral-sh/uv#22869

Classification: duplicate

## Summary

On Windows 11 Pro 25H2 (build 26200.9711, x86_64), the reporter says Smart App Control began blocking previously working managed Python installations and their virtual environments on October 23, 2026. Running a venv's `Scripts\python.exe --version` fails with `An Application Control policy has blocked this file`; Wing Pro is also unable to load the interpreter. The supplied signature check reports `NotSigned`. The affected environments were created with uv 0.12.5 or earlier; the reporter is now using uv 0.13.5. The original report describes Python 3.13.15, and supplied Windows Code Integrity events additionally identify managed CPython 3.14.2 as a blocked executable. In that recorded failure, the venv launcher is the calling process and the managed base interpreter is the blocked file.

The reporter installed signed Python.org CPython, set `UV_PYTHON_PREFERENCE=only-system`, and rebuilt the environments. Those environments run successfully and report valid signatures while remaining under the same user-profile directory. This makes the report more specific than a generic restriction on executing files under the user profile.

astral-sh/python-build-standalone#89 is the canonical signing request. astral-sh/uv#6584 reports related policy blocks, astral-sh/uv#10288 discusses antivirus effects on unsigned venv executables, and closed astral-sh/uv#10336 covers signing uv itself.

## Report components

- Managed-interpreter execution: Windows Smart App Control blocks Python installed under `C:\tools\uv\python`; the requested outcome is usable managed CPython with the policy enabled.
- Virtual-environment execution: invoking `Scripts\python.exe` fails directly and from an IDE; signed replacement environments work in the same location. The original signature check reports the launcher as unsigned, while the supplied Code Integrity event specifically records a block of the managed base interpreter launched by that process. It does not establish an independent block of the launcher itself.
- Distribution signing: provide trusted Authenticode signatures for the Python binaries supplied through managed installations, including their venv launchers.
- Documentation and interpreter selection: explain the compatibility limitation and how to select a signed system Python. The interpreter-selection capability already exists.
- Diagnostics: consider warning when a managed Windows interpreter is unsigned. No matching existing warning implementation or dedicated tracking issue was established.

## Draft response

Signing the Python distributions uv downloads is tracked in astral-sh/python-build-standalone#89. uv uses those CPython builds and normally copies their Windows venv launchers into new environments. Please add your Smart App Control reproduction there so the interpreter-signing discussion stays together.

astral-sh/uv#10336 was resolved for uv's own release executables in 0.12.12; that does not sign managed Python installations or replace executables in existing environments. Your workaround of selecting signed Python.org builds with `UV_PYTHON_PREFERENCE=only-system` and rebuilding the environments is consistent with uv's supported interpreter selection. `--no-managed-python` is the equivalent command-line option. Compatibility documentation and an unsigned-interpreter warning would require additional uv-side work.

## Classification

The core request for signed managed CPython binaries is already tracked by open astral-sh/python-build-standalone#89, which uv maintainers reference for Python-distribution signing. The Smart App Control reproduction and venv impact strengthen that existing request; documentation and warning suggestions are secondary follow-ups. No previously fixed managed-Python signing bug was identified: astral-sh/uv#10336 and uv 0.12.12 concern uv's own distributed executables.

## Related

- astral-sh/python-build-standalone#89 — Any plans on code signing the binary releases? (issue, open). Canonical upstream request for signed Python distributions, including Windows signing. uv documents that its managed CPython builds come from python-build-standalone, and a maintainer explicitly linked this discussion from astral-sh/uv#10336. It covers the underlying signing request affecting both the interpreter and its supplied venv launchers.
- astral-sh/uv#6584 — Group Policy Blocks UV Tool from Accessing Python Interpreter (issue, open). Reports Windows policy blocking a venv's Scripts\python.exe during uv add/run; an April 2026 comment also reports Smart App Control blocking Python executables. The original report concerns restricted locations, whereas this reporter rebuilt a working signed environment in the same location. Related symptom, but the upstream signing request is a closer canonical match.
- astral-sh/uv#10336 — Sign published executables for Windows (issue, closed). Closed when uv 0.12.12 introduced signing for uv release executables and binaries in uv/uv_build wheels. Its maintainer comments distinguish the upstream Python-signing discussion. That release did not establish a fix for managed CPython or existing venv executables, so this report is not evidence of a regression of that fix.
- astral-sh/uv#10288 — Windows AV software causes 0.5 second delay for unsigned binaries (issue, open). Discusses unsigned Windows venv Python executables, signing python-build-standalone binaries, and better behavior with official Python installers. The observed failure is antivirus startup latency rather than an execution block; it provides related signing context rather than an exact duplicate.

## Windows Code Integrity evidence

The reporter supplied event fields in the October 23, 2026 comment (comment ID 6369049504) on astral-sh/uv#22869. These are reported log excerpts; the raw event records and executable have not been independently examined.

| Field | Reported value |
| --- | --- |
| Log | `Microsoft-Windows-CodeIntegrity/Operational` |
| Enforcement event | Event ID `3077`, at `2026-10-23T15:04:48Z` |
| Blocked file | `<volume>\Tools\uv\python\cpython-3.14.2-windows-x86_64-none\python.exe` |
| Calling process | `<volume>\Users\<user>\.venvs\automation-lib\Scripts\python.exe` |
| Policy | `VerifiedAndReputableDesktop` |
| Requested / validated signing level in Event 3077 | `2` / `1` |
| Status | `0xc0e90002` |
| File version | `3.14.2150.1013` |
| User Writeable | `false` |
| SHA256 Hash | `1EBD3316C9963746F6EC7430B5336EC3420ED00A84A784160D5A0D62DDAAB052` |

The reporter also supplies Event ID `3089` with `TotalSignatureCount: 0`, `SignatureType: 0`, `ValidatedSigningLevel: 0`, unknown publisher and issuer, and `KnownRoot: 0`. They report that its Activity ID matches Event 3077 and its SHA-1 Authenticode hash, `F46C63CFC393771869B0E22575B1B67F80042581`, matches the corresponding hash in that block event. The excerpt does not include the Activity ID itself.

Microsoft's [App Control debugging and troubleshooting documentation](https://learn.microsoft.com/en-us/windows/security/application-security/application-control/app-control-for-business/operations/appcontrol-debugging-and-troubleshooting), reviewed for this update, identifies Event 3077 as an enforcement block and explains correlation with Event 3089 through Activity ID and file hashes. Requested signing level `2` with validated level `1` means Windows treated the file as unsigned; that pair alone does not distinguish a missing signature from an invalid or policy-disallowed signature. The documented meaning of `TotalSignatureCount: 0` in the correlated Event 3089 is that no signatures were detected for the blocked file.

Taken together, the supplied fields strengthen the evidence for unsigned managed CPython as the blocked target, extend the reported affected versions to Python 3.14.2, and locate this failure at the venv launcher's attempt to start the base interpreter. They do not explain why enforcement began on October 23 or establish a uv regression. The existing duplicate classification and signed-system-Python workaround remain applicable.

## Supporting evidence

- `docs/concepts/python-versions.md:450` identifies python-build-standalone as the source of managed CPython distributions. In astral-sh/uv#10336, maintainer zanieb linked astral-sh/python-build-standalone#89 on January 7, 2025. The upstream issue remains open and explicitly discusses pre-signed binaries and Windows signing.

- `crates/uv-virtualenv/src/virtualenv.rs:344` calls `copy_launcher_windows` on the ordinary Windows interpreter path. The function at line 843 copies interpreter-provided venv shims. Its second choice, at line 865, handles the `venvlauncher.exe`/`venvwlauncher.exe` names introduced in Python 3.13. This supports treating the normal venv launcher as part of the selected Python distribution; it does not independently verify the reporter's particular files.

- `docs/concepts/python-versions.md:396` documents `--no-managed-python`, and line 421 states that `only-system` is equivalent. `crates/uv-static/src/env_vars.rs:224` documents `UV_PYTHON_PREFERENCE`. These establish the supported selection mechanism; the successful workaround and signature measurements come from the reporter.

- The uv 0.12.12 release, published September 9, 2026, says the executables in uv's macOS/Windows release archives and uv/uv_build wheels are signed. Release evidence: https://github.com/astral-sh/uv/releases/tag/0.12.12. The same day, zanieb closed astral-sh/uv#10336 with that release announcement. Neither establishes signed managed CPython or automatic replacement of existing venv launchers.

- astral-sh/uv#6584 originally concerns location-based execution restrictions, but its April 7, 2026 comment also reports Smart App Control blocking Python executables. No signature comparison in that thread establishes that its original mechanism is identical to this report.

- The report establishes an execution failure and supplies a signed-versus-unsigned comparison. The reported correlated Code Integrity events additionally identify managed CPython 3.14.2 as the blocked target and report no detected signatures, as detailed above. The exact reason blocking began on October 23 is unconfirmed. There is no evidence here of a uv code regression, a prior managed-CPython signing fix, or a guaranteed delivery date for upstream signing.

## Inspected alternatives

astral-sh/uv#15022 and merged astral-sh/uv#15068 concern generated entrypoint executables failing to locate their payload after code signing; this report concerns unsigned CPython/venv executables rejected by Windows. The merged PR, dated November 9, 2025, makes those entrypoints robust to signing but does not distribute signed CPython. astral-sh/uv#18280 was an earlier release-binary signing proposal closed without merging on September 30, 2026. astral-sh/uv#10079 concerns detection of the uv installer. astral-sh/uv#14401 and astral-sh/uv#18000 concern execution from restricted cache/user-profile locations; the new report's working signed environments remain in the same location.

## Search coverage

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separate literal searches covered Smart App Control, the exact Application Control error, NotSigned, Authenticode, unsigned executables, python.exe/venvlauncher, signed system Python, documentation, and warnings. Conceptual searches covered group-policy blocks, AppLocker, WDAC, SmartScreen, signing, and interpreter selection, without the incidental IDE, Python version, or Windows build. Reviewed candidate comments and followed the maintainer link to astral-sh/python-build-standalone#89. Fix-oriented review covered the uv 0.12.12 release, closed astral-sh/uv#10336, unmerged astral-sh/uv#18280, and merged astral-sh/uv#15068. Ruled out astral-sh/uv#15022 and astral-sh/uv#15068 as the same problem: they fix entrypoint breakage after signing, not missing interpreter signatures. Also ruled out installer detection in astral-sh/uv#10079 and path-policy reports astral-sh/uv#14401 and astral-sh/uv#18000 as canonical matches. No separate matching documentation or unsigned-interpreter warning issue was found.

## Maintainer next step

Centralize the Python-distribution signing request in astral-sh/python-build-standalone#89 and carry over this concrete Smart App Control reproduction, including the managed CPython 3.14.2 path, binary hashes, and correlated enforcement/signature event fields. Investigation should distinguish the blocked base interpreter from the venv process that launched it. Keep the uv documentation and warning suggestions visible as possible follow-ups; no implementation or schedule is established. The reporter already has a working supported workaround, so an additional generic reproduction request is unnecessary.
