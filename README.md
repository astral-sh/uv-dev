# macOS 26: launchd-spawned uv-managed Python hangs at interpreter startup with syspolicyd 'Unable to initialize qtn_proc: 3' loop (distinct from astral-sh/uv#16726 SIGKILL)

Issue: astral-sh/uv#22915

Classification: bug

## Summary

The reporter describes uv-managed CPython hanging before executing a bare `python -c 'print(1)'` when launched by a macOS LaunchAgent. Direct execution, execution through `/bin/sh -c`, and execution through `uv run` reportedly fail under launchd. The identical interpreter works from a terminal, including with an empty environment and stdin redirected from `/dev/null`; Apple’s `/usr/bin/python3` works under launchd.

Reported environment: Homebrew uv 0.12.13, macOS 26 / Darwin 25.6.0 on arm64, and managed CPython 3.13.14 and 3.12.14. Prefect 3.6.29 was the original context, but the minimal example has no application dependencies.

The reported running-process sample ends in `Py_InitializeFromConfig → _PyConfig_InitPathConfig → getpath_readlines`. During the hang, syspolicyd repeatedly logs `Unable to initialize qtn_proc: 3` and `dispatch_mig_server returned 268435459`; killing Python reportedly stops the loop. A fresh 3.12 installation instead remains at launchd’s `spawn scheduled` state with last exit `78: EX_CONFIG`. These are separate observations; the evidence does not yet show whether their mechanisms match.

Re-signing the interpreter and shared libraries, attempting to remove provenance metadata, setting `ProcessType=Interactive`, and setting `PYTHONHOME` reportedly did not help. The reporter restored the pristine installation after re-signing affected keychain trust. Terminal-origin `nohup` execution is the reported workaround, not an independently verified fix.

astral-sh/uv#16726 and its unmerged signing proposal astral-sh/uv#17123 are closest. astral-sh/uv#17371 and astral-sh/uv#18596 concern adjacent cold-start scan overhead. No established duplicate or shipped fix for the launchd startup hang was found.

Reproduction outcome: **needs_more_information**. The available runner is Linux x86_64, so the macOS launchd/syspolicyd trigger could not be exercised. The reported hang remains unverified; this is not a successful negative reproduction.

## Draft response

We could not independently validate the macOS-specific behavior on the available Linux runner. Your reported bare-interpreter reproduction and terminal-versus-launchd comparison support keeping this separate for now. astral-sh/uv#16726 also contains an extension-import hang report, but bare print works in that example and re-signing reportedly helps. The proposed Python re-signing change, astral-sh/uv#17123, was closed without merging. The signature metadata and syspolicyd messages do not yet establish the cause of this hang.

Please attach the complete minimal LaunchAgent plist, including its Label, environment and working-directory settings, the exact bootstrap command and output, the full process sample from the hung interpreter, and a short timestamp-matched syspolicyd log excerpt. Please include the interpreter installation command and python-build-standalone build identifier (the installation’s BUILD file, if present), so a fresh installation can use the same artifact. Please also include the exact macOS version/build from `sw_vers`, the SDK version from `xcrun --show-sdk-version`, and whether the machine has MDM policies or third-party antivirus/EDR. For the fresh 3.12 case, include the relevant `launchctl print` state so we can distinguish failure to spawn from a running interpreter blocked during initialization.

## Classification

Classify this as a reported runtime bug: the expected result is for the managed interpreter to print and exit under launchd as it does from a terminal. The available investigation has not independently observed the reported failure or established its cause. Existing reports overlap in platform and security-related observations, but do not establish the same initialization failure: the closest hang report requires extension imports, and re-signing helps there but fails here. Keep this separate pending evidence of a shared mechanism. No previously merged fix for this failure was found, so a regression is not established.

The issue concerns a reported runtime failure, not a request for a new feature or merely an explanation of expected behavior. The report provides useful controls, but no macOS host is available for a behavioral reproduction. Correlated syspolicyd activity and ad-hoc signing are observations, not proof that Gatekeeper, provenance, an SDK version, or endpoint software caused the hang.

The reporter’s description of astral-sh/uv#16726 as a launchd SIGKILL report is not established by the inspected thread. Its launchd example schedules a re-signing workaround. Its later hang example loads native extensions and explicitly shows bare print succeeding. These differences support retaining a separate report without asserting that the two failures cannot share a cause.

## Reproduction

**Outcome: needs_more_information.** The essential platform is unavailable. This does not contradict the reporter’s result, establish a uv defect, or justify a `not_reproducible` conclusion.

### Reported configuration and expected result

- Homebrew uv 0.12.13; macOS 26 / Darwin 25.6.0; arm64.
- Managed CPython 3.13.14 and 3.12.14, described as ad-hoc/linker-signed with `Identifier=-`.
- A GUI-domain LaunchAgent executes the absolute managed-interpreter path with `-c` and `print('x')`. No project, virtual environment, dependency group, Prefect installation, or package dependency is needed for this direct-interpreter case.
- Expected: stdout contains `x` and the process exits successfully. Reported actual behavior: 3.13 hangs before printing, with the sampled initialization stack and correlated syspolicyd loop described above. The fresh 3.12 `spawn scheduled` / `78: EX_CONFIG` observation must be assessed separately.

### Checks performed in this environment

```console
$ uname -srm
Linux 6.17.0-1022-azure x86_64
$ command -v uv
/opt/hostedtoolcache/uv/0.13.5/x86_64/uv
$ uv --version
uv 0.13.5 (x86_64-unknown-linux-gnu)
$ command -v launchctl
$ command -v sw_vers
```

Both macOS command lookups returned no path. The installed uv executable’s `python install --help` was also inspected. No managed Python installation, launchd job, or startup experiment was run: Linux cannot reproduce macOS process lineage, code-signing policy, or syspolicyd behavior. Ordinary Linux startup would not exercise the distinguishing launchd/security-policy trigger. No affected-versus-good version comparison was possible; the report does not identify a last known-good uv/Python/macOS combination or establish an upgrade regression.

### Minimal macOS fixture, reconstructed but not executed

The supplied XML is a fragment, so a complete plist needs its outer dictionary and a unique `Label`. A standalone, dependency-free fixture was generated at `/tmp/uv-22915-j3c4mask/probe.plist` and successfully round-tripped through Python’s `plistlib`. This validates its XML structure only, not acceptance by launchd or Python startup. Its interpreter path is deliberately a placeholder and must be replaced with the absolute path of the affected interpreter. All generated files and output paths are under the temporary directory `/tmp/uv-22915-j3c4mask`.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>org.astral.uv.issue22915.probe</string>
	<key>ProgramArguments</key>
	<array>
		<string>/absolute/path/to/cpython-3.13.14-macos-aarch64-none/bin/python3.13</string>
		<string>-c</string>
		<string>print('x')</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>StandardOutPath</key>
	<string>/tmp/uv-22915-j3c4mask/probe.log</string>
	<key>StandardErrorPath</key>
	<string>/tmp/uv-22915-j3c4mask/probe.err</string>
</dict>
</plist>
```

On an affected Mac, create an equivalent plist in a fresh temporary directory with absolute interpreter and log paths. The relevant invocation is:

```sh
launchctl bootstrap "gui/$(id -u)" "$probe_dir/probe.plist"
launchctl print "gui/$(id -u)/org.astral.uv.issue22915.probe"
```

Compare against direct execution of that same interpreter, including the reporter’s empty-environment control:

```sh
"$python_path" -c "print('x')"
env -i "$python_path" -c "print('x')" </dev/null
```

Capture the job state and, if a Python PID exists, its process sample and timestamp-matched syspolicyd messages before stopping the temporary job. Remove that job with `launchctl bootout "gui/$(id -u)/org.astral.uv.issue22915.probe"`. These macOS commands were not executed here. A separate fresh-install trial would need temporary `UV_PYTHON_INSTALL_DIR` and cache paths plus `uv python install --no-bin`; it must record the selected build rather than assume that a patch-version match recreates the reporter’s interpreter artifact or security metadata.

### Existing test coverage inspected

Searched `crates/uv/tests/`, including `crates/uv/tests/it/`, and `crates/uv-client/tests/it/` for launchd, launchctl, LaunchAgent, qtn_proc, syspolicyd, getpath_readlines, EX_CONFIG, signing, and quarantine-related coverage. No test exercising this startup trigger or diagnostic loop was found. Python installation integration tests reside in `crates/uv/tests/python/` in this checkout.

- `crates/uv/tests/python/python_install.rs::python_install`: uses temporary managed-Python and executable directories, installs the current default Python (3.15 in this checkout), verifies the executable link, then directly runs `-c "import subprocess; print('hello world')"` and snapshots exit code 0 and `hello world`. It covers ordinary startup from the test process, not a LaunchAgent.
- `crates/uv/tests/python/python_install.rs::python_install_patch_dylib`: macOS-only; installs Python 3.13.1 with `--preview`, runs `otool -D` on libpython, and snapshots the rewritten install name. It does not start Python or test code-signing policy or launchd.
- Both belong to the `test-python-managed` feature-gated module in `crates/uv/tests/python/main.rs`. Their setup and assertions were read; they were not run on this Linux host and are not regression coverage for the reported hang.

### Evidence needed to continue

The primary requirement is a macOS 26 arm64 host with a GUI launchd session, ideally the reporter’s affected machine. Obtain the complete plist (including `Label`, any `EnvironmentVariables`, `WorkingDirectory`, and other launch options), exact commands and bootstrap output, full `sw_vers` version/build, interpreter installation command and build identifier, a complete hung-process sample, and a short corresponding syspolicyd log excerpt. Also record SDK version and whether MDM or antivirus/EDR is present, without assuming these cause the failure. For 3.12, capture `launchctl print` and whether an interpreter PID exists to distinguish spawn failure from interpreter initialization. No project configuration is required for the reported bare-interpreter reproduction.

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
- Local `changelogs/0.12.x.md` confirms that uv 0.12.12 introduced signing of uv/uv-build release artifacts; uv 0.12.13 lists no managed-CPython startup fix. `CHANGELOG.md` places the stable signing-identifier change in uv 0.13.2. These release notes do not establish signing of managed CPython or a fix for this launchd hang.
- The current `crates/uv-python-managed/download-metadata.json` maps macOS arm64 CPython 3.13.14 to python-build-standalone build `20260805` and CPython 3.12.14 to build `20260929`. These are current catalog entries, not verification of the artifacts installed on the reporter’s machine.

## Search scope and excluded candidates

The prior issue-context investigation searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. The reproduction pass retained those findings and inspected local source, release notes, and test assertions; it did not repeat the GitHub searches. Separate literal searches covered qtn_proc, dispatch_mig_server, getpath_readlines, _PyConfig_InitPathConfig, EX_CONFIG, spawn scheduled, launchd, launchctl, and LaunchAgent; exact diagnostic fragments found only the new issue. Conceptual searches covered Python hangs/startup/freezes, daemons, macOS cold starts, and area:macos startup reports, including queries without platform or version restrictions. Separate cause-oriented searches covered syspolicyd, Gatekeeper, provenance, codesign, keychain signing/ACLs, and install_name_tool. PR searches covered signing, Mach-O, launchd, and interpreter startup; keyword searches returned no candidates, so referenced PRs were inspected directly. Followed comments and duplicate chains through astral-sh/uv#16003, astral-sh/uv#16297, astral-sh/uv#18577, and astral-sh/uv#20792. Ruled out astral-sh/uv#10613 (transient terminal/Homebrew-Python hang), astral-sh/uv#14843 (installation-time binary corruption), and astral-sh/uv#22627 with merged astral-sh/uv#22648 (uv/uvx signing identity, not managed Python). No matching shipped startup fix was established.

Additional launchd matches were inspected and excluded: astral-sh/uv#14123 concerns replacing uv’s parent process with exec and explicitly reports no concrete failure; astral-sh/uv#12869 concerns a missing extension caused by its CMake installation path. astral-sh/uv#14508 concerns interpreter-query hangs on Windows, without the macOS launchd trigger or diagnostics.

Search results and linked discussions establish no existing report of the same qtn_proc loop or EX_CONFIG variant. PR keyword searches returned no candidates, so the inspected PR set comes from issue/comment references and the local changelog; this limits claims of completeness.

## Next diagnostic step

Obtain an affected macOS host and collect the artifacts requested above before deciding whether to centralize the report or investigate the interpreter distribution separately. In particular, distinguish a running 3.13 process blocked in initialization from the 3.12 launchd job waiting to spawn. Do not repeat the already unsuccessful re-signing workaround or present hardlinks, an SDK update, or uv release signing as a confirmed fix.

The handoff combines the prior GitHub discussion review with local repository, test, release-note, and platform checks on October 25, 2026. The reconstructed plist passed a local XML round-trip only. No macOS reproduction or interpreter runtime validation was performed. The outcome remains needs_more_information. The checkout and GitHub were not modified.
