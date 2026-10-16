# `uv python install` does not repair a managed installation whose interpreter executable is missing

Issue: astral-sh/uv#22649

Classification: bug

## Summary

A managed Python installation remains discoverable after its interpreter executable is deleted while the installation directory and other files remain. The reporter demonstrates that `uv python install 3.12` exits successfully and reports an installation without restoring the executable. `uv run --no-project --managed-python --python 3.12` then fails with `Python interpreter not found`; the reporter also observes this with `uv sync` and reports fallback to system Python when managed-only selection is removed.

The reproduction removes `bin/python3.12` from the Linux installation or `python.exe` from the Windows installation. It reports Ubuntu 24.04 under WSL2 and Windows 11, Python 3.12.15, uv 0.13.1, and the same behavior on uv 0.12.23 and main at `3d9ce3cb`. Explicit `--reinstall`, or uninstall followed by install, restores the installation. Expected behavior is repair during a plain install and automatic download when a command needs that interpreter.

astral-sh/uv#22650 is the open proposed fix for both explicit installation repair and automatic recovery. astral-sh/uv#19622 and astral-sh/uv#19329, with its proposed fix astral-sh/uv#20309, concern adjacent but distinct failures. No earlier exact duplicate was found.

## Draft response

The current code confirms your diagnosis: installation matching does not check that the interpreter exists, and the download path reuses any existing installation directory. That explains why a plain install can report success while Python remains unusable. Your PR astral-sh/uv#22650 addresses both explicit installation and automatic recovery and remains open for review. For now, use `uv python install --reinstall 3.12` to restore the interpreter. This is separate from the junction failure in astral-sh/uv#19622 and the finalization race in astral-sh/uv#19329.

## Classification

The source accepts a matching managed installation without verifying its interpreter and returns AlreadyAvailable whenever the directory exists, explaining the failure to repair an unusable installation and the misleading success report. This is established incorrect behavior. The direct fix, astral-sh/uv#22650, was created in response to this issue, so duplicate classification does not apply. The related junction, finalization, executable-naming, and virtual-environment fixes address different conditions; no regression of a previously fixed version of this bug was established.

The issue was opened on 2026-10-16 at 17:14:06 UTC; astral-sh/uv#22650 followed at 17:14:15 UTC and explicitly identifies this issue as the one it fixes. Its existence therefore records work on this bug rather than an earlier canonical discussion.

## Related

- astral-sh/uv#22650 — pull request, open: Reinstall managed Pythons whose executable is missing. Direct proposed fix, opened nine seconds after astral-sh/uv#22649 and explicitly closing it. The diff checks executable existence in installation matching, discovery, and download reuse, with separate tests for explicit installation and automatic recovery through uv run. It is not merged and does not make the originating issue a duplicate.
- astral-sh/uv#19622 — issue, open: `uv python install --reinstall` cannot recover a dangling minor-version-link junction on Windows: "Missing expected target directory for Python minor version link". Related managed-installation recovery failure, but the patch-directory interpreter remains executable and the Windows junction fails to resolve; even --reinstall fails. Later comments associate that case with MSIX filesystem virtualization. Here the interpreter itself is absent, Linux is also affected, and --reinstall succeeds.
- astral-sh/uv#19329 — issue, open: `uv python install` publishes managed Python before finalization, allowing `uv python find` to race and observe incomplete installs. Shares managed-installation discovery and completeness concerns, but tracks concurrent discovery before finalization and stale interpreter metadata. The new report removes the executable after installation completes and requires no concurrent process, so it is a separate bug.
- astral-sh/uv#20309 — pull request, open: fix: finalize managed Python installation before atomic rename. Proposed fix for astral-sh/uv#19329 moves finalization before publication. Its diff addresses the installation race, not executable loss from an existing installation. It remains unmerged and is neither a fix for this report nor evidence of a regression.

## Supporting evidence

Source inspection used checkout commit `64e2ac873ff0c589024453666c2b9a5bd89dc250` dated 2026-10-16. The relevant crates have been split since some older reports, so their historical source paths differ.

- `crates/uv-python-managed/src/managed.rs:198`: `find_all` enumerates installation directories and constructs installation records without checking interpreter existence.
- `crates/uv-python-commands/src/install.rs:99` and `crates/uv-python-commands/src/install.rs:541`: `InstallRequest::matches_installation` matches the installation key; the ordinary install branch accepts that match as satisfied.
- `crates/uv-python-managed/src/downloads.rs:500`: `fetch_from_url` returns `AlreadyAvailable` when `!reinstall && path.is_dir()`. An existing directory therefore prevents a fresh download even when its interpreter is absent.
- `crates/uv-python-discovery/src/discovery.rs:220`: managed discovery filters version, platform, and requested build, then yields an interpreter path without testing executable existence.
- `crates/uv-python-discovery/src/installation.rs:342`: automatic installation accepts either `AlreadyAvailable` or `Fetched`, finalizes the installation, and queries its interpreter. The download shortcut also affects this recovery path.
- `crates/uv-python-managed/src/managed.rs:518`: canonical executable setup creates aliases or returns immediately when the source and destination are identical; it does not restore the missing underlying interpreter.
- `crates/uv-python-interpreter/src/interpreter.rs:1344`: cached interpreter queries can produce the missing-interpreter error during path canonicalization or metadata access. The reporter's narrower claim that the error arises only when spawning is not necessary to establish the bug.

The diff of astral-sh/uv#22650 adds an executable-existence check to managed installation matching, discovery, and download reuse. It adds separate `python_install_missing_executable` and `python_install_automatic_missing_executable` integration tests. Its author reports that they fail on the earlier main revision and that Linux tests and clippy pass with the fix; those execution results were not independently verified here. There are no review comments or approvals on that PR at inspection time.

Existing `python_reinstall` coverage exercises an explicit reinstall, and `python_install_broken_link` repairs an external executable symlink. Those are different from deleting the underlying interpreter while retaining its installation directory. The existing `python_install` test module is gated by `test-python-managed` in `crates/uv/tests/python/main.rs:8`.

## Search scope and ruled-out candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered 'Python interpreter not found', 'missing executable', 'python install', 'AlreadyAvailable', 'matches_installation', cpython-3.12.15, and 0.12.23. Separate conceptual searches covered repair, corrupt/broken/incomplete managed installations, deleted interpreters, and redownload/re-download; also used area:uv-python. Checked historical fixes and followed candidate comments and timelines. PR text searches returned no matches even for known candidates, so supplemented them with the latest 100 all-state PRs and direct inspection of linked PRs. Ruled out astral-sh/uv#8298 and merged astral-sh/uv#8310 (Windows free-threaded executable naming), astral-sh/uv#4933 and merged astral-sh/uv#4935 (recreating broken virtual environments), astral-sh/uv#8028 (tool-environment repair), and astral-sh/uv#11355 (a host .venv mounted into Docker). No earlier exact duplicate or previously merged fix for this behavior was established.

The report was decomposed before searching into: successful-but-ineffective explicit installation; failed automatic recovery in run/sync; system-interpreter fallback masking the broken installation; retained installation directory with a missing canonical executable; and the separately proposed directory/key-validation mechanism. Literal error and identifier searches were kept separate from searches for incomplete publication, junction failures, and corruption.

The strongest historical alternatives do not establish a regression:

- astral-sh/uv#8298 concerns Windows Python 3.13 free-threaded distributions containing `python3.13t.exe` instead of `python.exe`. Maintainer comments point to astral-sh/uv#8310, merged on 2024-10-17, which adds executable-name compatibility. This differs from deleting the canonical interpreter from an otherwise installed Python 3.12 distribution.
- astral-sh/uv#4933 was closed by astral-sh/uv#4935, merged on 2024-07-09. That change recreates project/tool virtual environments after their base interpreter is removed; it does not validate or replace the remaining managed interpreter directory.
- astral-sh/uv#8028 requests repair of tool environments after Python upgrades, while astral-sh/uv#11355 was resolved by avoiding a host virtual environment mounted over the container's environment.

PR search results were incomplete: text searches returned empty results while direct issue timelines and PR listings exposed relevant PRs. The handoff therefore does not claim an exhaustive negative search over all historical PRs.

## Next step and verification limits

Review astral-sh/uv#22650 for both explicit repair and automatic recovery, including platform results for deletion of the actual canonical interpreter. Retain the distinction from the junction and publication-race work. The immediate documented workaround is `uv python install --reinstall 3.12`.

This handoff is based on read-only GitHub inspection, current source, and the proposed diff. No reproduction, build, or test was run, and no fix is claimed to be released. GitHub was not changed, and no checkout files were modified by this triage.
