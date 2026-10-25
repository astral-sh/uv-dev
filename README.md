# macOS 26: launchd-spawned uv-managed Python hangs at interpreter startup with syspolicyd 'Unable to initialize qtn_proc: 3' loop (distinct from astral-sh/uv#16726 SIGKILL)

Issue: astral-sh/uv#22915

Classification: bug

## Summary

The reporter describes uv-managed CPython hanging before executing a bare `python -c 'print(1)'` when launched by a macOS LaunchAgent. Direct execution, execution through `/bin/sh -c`, and execution through `uv run` reportedly fail under launchd. The identical interpreter works from a terminal, including with an empty environment and stdin redirected from `/dev/null`; Apple’s `/usr/bin/python3` works under launchd.

Reported environment: Homebrew uv 0.12.13, macOS 26 / Darwin 25.6.0 on arm64, and managed CPython 3.13.14 and 3.12.14. Prefect 3.6.29 was the original context, but the minimal example has no application dependencies.

The reported running-process sample ends in `Py_InitializeFromConfig → _PyConfig_InitPathConfig → getpath_readlines`. During the hang, syspolicyd repeatedly logs `Unable to initialize qtn_proc: 3` and `dispatch_mig_server returned 268435459`; killing Python reportedly stops the loop. A fresh 3.12 installation instead remains at launchd’s `spawn scheduled` state with last exit `78: EX_CONFIG`. These are separate observations; the evidence does not yet show whether their mechanisms match.

Re-signing the interpreter and shared libraries, attempting to remove provenance metadata, setting `ProcessType=Interactive`, and setting `PYTHONHOME` reportedly did not help. The reporter restored the pristine installation after re-signing affected keychain trust. Terminal-origin `nohup` execution is the reported workaround, not an independently verified fix.

astral-sh/uv#16726 and its unmerged signing proposal astral-sh/uv#17123 are closest. astral-sh/uv#17371 and astral-sh/uv#18596 concern adjacent cold-start scan overhead. No established duplicate or shipped fix for the launchd startup hang was found.

## Draft response

Your bare-interpreter reproduction and the terminal-versus-launchd comparison support keeping this separate for now. astral-sh/uv#16726 also contains an extension-import hang report, but bare print works in that example and re-signing reportedly helps. The proposed Python re-signing change, astral-sh/uv#17123, was closed without merging. The signature metadata and syspolicyd messages do not yet establish the cause of this hang.

Please attach the complete minimal LaunchAgent plist, the full process sample from the hung interpreter, and a short timestamp-matched syspolicyd log excerpt. Please also include the exact macOS version/build from `sw_vers`, the SDK version from `xcrun --show-sdk-version`, and whether the machine has MDM policies or third-party antivirus/EDR. For the fresh 3.12 case, include the relevant `launchctl print` state so we can distinguish failure to spawn from a running interpreter blocked during initialization.

## Classification

A managed interpreter hanging before executing a bare print command under launchd is incorrect behavior. Existing reports overlap in platform and security-related observations, but do not establish the same initialization failure: the closest hang report requires extension imports, and re-signing helps there but fails here. Keep this separate pending evidence of a shared mechanism. No previously merged fix for this failure was found, so a regression is not established.

The issue concerns a reported runtime failure, not a request for a new feature or merely an explanation of expected behavior. The report provides useful controls, but this investigation did not reproduce the problem on macOS. Correlated syspolicyd activity and ad-hoc signing are observations, not proof that Gatekeeper, provenance, an SDK version, or endpoint software caused the hang.

The reporter’s description of astral-sh/uv#16726 as a launchd SIGKILL report is not established by the inspected thread. Its launchd example schedules a re-signing workaround. Its later hang example loads native extensions and explicitly shows bare print succeeding. These differences support retaining a separate report without asserting that the two failures cannot share a cause.

## Related

- astral-sh/uv#16726 — UV Python SIGKILL Issue on macOS (issue, open). Closest managed-Python execution failure discussion. It reports SIGKILL and a later hang when importing native extensions, while bare print succeeds in that example. Re-signing reportedly helps those cases; here bare interpreter initialization hangs specifically under launchd despite re-signing. A maintainer reproduced the signature/provenance metadata without reproducing the failure, so that metadata does not establish a shared cause.
- astral-sh/uv#17123 — fix: add ad-hoc code signing for Python binaries on macOS (pull request, closed). Proposed installation-time Python re-signing for astral-sh/uv#16726 and astral-sh/uv#16003. Closed without merging on December 13, 2025, contrary to a later issue comment claiming it shipped. It does not establish a previous fix or regression, and the reporter already tried its re-signing approach unsuccessfully.
- astral-sh/uv#17371 — first run after `uv sync` is ~20x slower in MacOs; subsequent runs fast (not reproduced on Linux / miniconda) (issue, open). Canonical macOS cold-start discussion: maintainers closed the explicit syspolicyd-rescan report astral-sh/uv#18577 into this issue. It concerns finite delays loading dependencies after environment recreation, reduced by hardlinks. That differs from an indefinite launchd-only hang in a bare managed interpreter.
- astral-sh/uv#18596 — Prefer hardlinks over reflinks for Mach-O executables to reduce security scanning overhead (pull request, open). Proposed fix for astral-sh/uv#17371 preserves Mach-O file inodes during package installation to avoid repeated security scans. It remains unmerged and addresses environment recreation/import overhead, not the reported direct launchd invocation of an already installed interpreter.

## Supporting evidence and release status

- In astral-sh/uv#16726, maintainer zsol reported on November 13, 2025 that the same xattr, codesign, and spctl output occurred on working installations. The March 27, 2026 extension-import hang is a reporter observation, including its claimed mechanism; it is not a maintainer-confirmed diagnosis. On October 5, 2026, woodruffw requested SDK and MDM/AV/EDR details. Those details remain missing here.
- astral-sh/uv#16003 was closed into astral-sh/uv#16726. astral-sh/uv#16297 was closed after a reporter recovered by reinstalling/upgrading Python; the maintainer explicitly did not know why it helped. Neither closure establishes a released correction for this hang.
- astral-sh/uv#17123 has no merge date and was closed on December 13, 2025. A later assertion that it had shipped is contradicted by the PR’s recorded state.
- astral-sh/uv#18577 explicitly reports negligible bare-interpreter overhead and finite native-extension scan delays. A maintainer closed it into astral-sh/uv#17371. The linked hardlink proposal astral-sh/uv#18596 remains open.
- astral-sh/uv#14870 establishes signing of Astral-distributed uv/uvx executables starting in uv 0.12.12, and clarifies that Homebrew builds its own binaries. This is not evidence that downloaded CPython interpreters received that signing change. The historical release-signing proposal astral-sh/uv#18280 was closed without merging.
- The merged astral-sh/uv#22648 changes stable identities for uv, uvx, and uv-build release binaries. It does not change the managed Python interpreter or establish a fix for the reported keychain side effect.
- Repository documentation at `docs/concepts/python-versions.md:450` identifies python-build-standalone as the source of managed CPython distributions. `crates/uv-python-managed/src/managed.rs:627` and `crates/uv-python-managed/src/macos_dylib.rs:8` show installation-time dylib install-name patching through `install_name_tool`. This source inspection establishes installation behavior only; it does not explain the launchd failure.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Separate literal searches covered qtn_proc, dispatch_mig_server, getpath_readlines, _PyConfig_InitPathConfig, EX_CONFIG, spawn scheduled, launchd, launchctl, and LaunchAgent; exact diagnostic fragments found only the new issue. Conceptual searches covered Python hangs/startup/freezes, daemons, macOS cold starts, and area:macos startup reports, including queries without platform or version restrictions. Separate cause-oriented searches covered syspolicyd, Gatekeeper, provenance, codesign, keychain signing/ACLs, and install_name_tool. PR searches covered signing, Mach-O, launchd, and interpreter startup; keyword searches returned no candidates, so referenced PRs were inspected directly. Followed comments and duplicate chains through astral-sh/uv#16003, astral-sh/uv#16297, astral-sh/uv#18577, and astral-sh/uv#20792. Ruled out astral-sh/uv#10613 (transient terminal/Homebrew-Python hang), astral-sh/uv#14843 (installation-time binary corruption), and astral-sh/uv#22627 with merged astral-sh/uv#22648 (uv/uvx signing identity, not managed Python). No matching shipped startup fix was established.

Additional launchd matches were inspected and excluded: astral-sh/uv#14123 concerns replacing uv’s parent process with exec and explicitly reports no concrete failure; astral-sh/uv#12869 concerns a missing extension caused by its CMake installation path. astral-sh/uv#14508 concerns interpreter-query hangs on Windows, without the macOS launchd trigger or diagnostics.

Search results and linked discussions establish no existing report of the same qtn_proc loop or EX_CONFIG variant. PR keyword searches returned no candidates, so the inspected PR set comes from issue/comment references and the local changelog; this limits claims of completeness.

## Next diagnostic step

Collect the artifacts requested in the draft before deciding whether to centralize the report or investigate the interpreter distribution separately. In particular, distinguish a running 3.13 process blocked in initialization from the 3.12 launchd job waiting to spawn. Do not repeat the already unsuccessful re-signing workaround or present hardlinks, an SDK update, or uv release signing as a confirmed fix.

The handoff is based on repository source and GitHub discussions inspected on October 25, 2026. No macOS reproduction or runtime validation was performed.
