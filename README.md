# `uv python install` does not repair a managed installation whose interpreter executable is missing

Issue: astral-sh/uv#22649

Classification: bug

## Summary

Reproduced on Ubuntu 24.04.5 LTS x86_64 using the installed uv 0.13.0 and managed CPython 3.12.15. Deleting only the managed installation's `bin/python3.12` leaves an installation that `uv python install 3.12` reports as successfully installed without restoring its interpreter. `uv run --no-project --managed-python --python 3.12` fails with `Python interpreter not found` both before and after the ordinary install. Explicit `--reinstall` restores the interpreter and allows the same run command to succeed.

The report describes Ubuntu 24.04 under WSL2 and Windows 11, Python 3.12.15, uv 0.13.1, and the same behavior on uv 0.12.23 and main at `3d9ce3cb`. The expected behavior is repair during a plain install and automatic download when a command needs the interpreter. Windows, the other uv versions, `uv sync`, and fallback to system Python were not independently exercised in this reproduction.

The existing related-work investigation identifies astral-sh/uv#22650 as the open proposed fix for explicit installation repair and automatic recovery. astral-sh/uv#19622 and astral-sh/uv#19329, with its proposed fix astral-sh/uv#20309, concern adjacent but distinct failures. No earlier exact duplicate was found in that investigation; GitHub status was not rechecked during reproduction.

## Draft response

Reproduced with uv 0.13.0 on Ubuntu 24.04 and Python 3.12.15: after deleting the managed interpreter, plain install exits 0 and reports success while the file remains missing, and managed-only `uv run` exits 2. `uv python install --reinstall 3.12` restores the file and the run command succeeds. The proposed fix is tracked in astral-sh/uv#22650. This differs from the junction failure in astral-sh/uv#19622 and the finalization race in astral-sh/uv#19329.

## Classification

Bug classification is supported by the observed successful installation report while the requested managed interpreter remains absent and unusable. Reproduction establishes this behavior and the explicit-reinstall workaround; it does not establish that every reported selection path behaves identically or independently validate the proposed fix.

The direct fix, astral-sh/uv#22650, was created in response to this issue, so duplicate classification does not apply. The related junction, finalization, executable-naming, and virtual-environment fixes address different conditions; no regression of a previously fixed version of this bug was established.

The issue was opened on 2026-10-16 at 17:14:06 UTC; astral-sh/uv#22650 followed at 17:14:15 UTC and explicitly identifies this issue as the one it fixes. Its existence therefore records work on this bug rather than an earlier canonical discussion.

## Reproduction

Outcome: **reproducible**.

### Environment and isolation

- Ubuntu 24.04.5 LTS, Linux `6.17.0-1022-azure`, x86_64; this runner is not the reported WSL2 or Windows environment.
- Installed executable on PATH: `/opt/hostedtoolcache/uv/0.13.0/x86_64/uv`; version output: `uv 0.13.0 (x86_64-unknown-linux-gnu)`.
- Downloaded installation: `cpython-3.12.15-linux-x86_64-gnu`. Its interpreter printed `Python 3.12.15` before deletion.
- A fresh directory, `/tmp/uv-22649-fjww7aua`, held the working directory, managed installation, bin directory, cache, temporary files, and XDG directories. No project files were needed.
- Subprocesses received an explicitly constructed environment containing only PATH, the isolated UV/XDG/TMPDIR settings, `UV_NO_CONFIG=1`, `NO_COLOR=1`, and `TERM=dumb`. The isolated bin directory was prepended to PATH. No inherited virtual environment or uv configuration was used.

### Minimal commands

The following is the core sequence, with all state redirected to a new temporary directory. The actual run used the installed uv executable's absolute path and captured each exit status separately.

```sh
repro_dir=$(mktemp -d /tmp/uv-22649-XXXXXX)
export UV_PYTHON_INSTALL_DIR="$repro_dir/python"
export UV_PYTHON_BIN_DIR="$repro_dir/bin"
export UV_CACHE_DIR="$repro_dir/cache"
export UV_NO_CONFIG=1
export XDG_CONFIG_HOME="$repro_dir/config"
export XDG_DATA_HOME="$repro_dir/data"
export XDG_CACHE_HOME="$repro_dir/xdg-cache"
export TMPDIR="$repro_dir/tmp"
export PATH="$repro_dir/bin:$PATH"
mkdir -p "$repro_dir/work" "$repro_dir/bin" "$TMPDIR"
cd "$repro_dir/work"

uv python install 3.12
interpreter="$UV_PYTHON_INSTALL_DIR/cpython-3.12.15-linux-x86_64-gnu/bin/python3.12"
"$interpreter" --version
rm "$interpreter"
uv run --no-project --managed-python --python 3.12 python -c "print('ok')"
uv python install 3.12
test -e "$interpreter"
uv run --no-project --managed-python --python 3.12 python -c "print('ok')"
uv python install --reinstall 3.12
test -e "$interpreter"
uv run --no-project --managed-python --python 3.12 python -c "print('ok')"
```

Run each command despite the expected failures; do not enable shell exit-on-error for this sequence. The executable path reflects the Python version resolved during this reproduction.

### Observed results

| Step | Exit status | Observation |
| --- | --- | --- |
| Initial install | 0 | Downloaded and installed Python 3.12.15. |
| Direct interpreter version check | 0 | Printed `Python 3.12.15`. |
| Delete the canonical interpreter | Successful file deletion | The installation directory remained; only its `bin/python3.12` file was removed. |
| Managed-only run after deletion | 2 | `error: Python interpreter not found at <temporary installation>/bin/python3.12`. |
| Ordinary install after deletion | 0 | Printed `Installed Python 3.12.15 in 40ms` and `+ cpython-3.12.15-linux-x86_64-gnu (python3.12)`; no download was reported. The interpreter still did not exist. |
| Managed-only run after ordinary install | 2 | The same missing-interpreter error. |
| Explicit reinstall | 0 | Downloaded Python again, reported `~ cpython-3.12.15-linux-x86_64-gnu (python3.12)`, and restored the interpreter. |
| Managed-only run after explicit reinstall | 0 | Printed `ok`. |

The filesystem checks were performed directly in the reproduction driver; the shell sequence above expresses them as `test -e`. The captured command arguments, exit statuses, stdout, and stderr are in `/tmp/uv-22649-fjww7aua/results.json` for the lifetime of this runner.

### Existing test coverage

Searched `crates/uv/tests/` and `crates/uv-client/tests/it/`, prioritizing the Python installation integration tests. Read the setup and assertions for these related tests in `crates/uv/tests/python/python_install.rs`:

- `python_reinstall`: explicitly reinstalls one or multiple versions and installs an absent version with `--reinstall`; it does not delete an installed interpreter or test ordinary repair.
- `python_install_automatic`: tests automatic download when Python is absent, reuse with downloads disabled, version requests, and ignoring a broken executable placed on the search path. It does not retain a managed installation directory while deleting its canonical interpreter.
- `python_install_broken_link`: creates an external bin symlink pointing to a nonexistent target before installation, then asserts that installation replaces the symlink with the managed interpreter target. It does not remove the interpreter from an existing installation.

No exact coverage of the retained-directory/missing-canonical-interpreter scenario was found in these existing tests. The module is gated by `test-python-managed` in `crates/uv/tests/python/main.rs:8`; `python_install_broken_link` is also Unix-only. The proposed tests in astral-sh/uv#22650 are separate from the checked-out tests and were not run here.

## Related

- astral-sh/uv#22650 — pull request, open: Reinstall managed Pythons whose executable is missing. Direct proposed fix, opened nine seconds after astral-sh/uv#22649 and explicitly closing it. The diff checks executable existence in installation matching, discovery, and download reuse, with separate tests for explicit installation and automatic recovery through uv run. It is not merged and does not make the originating issue a duplicate.
- astral-sh/uv#19622 — issue, open: `uv python install --reinstall` cannot recover a dangling minor-version-link junction on Windows: "Missing expected target directory for Python minor version link". Related managed-installation recovery failure, but the patch-directory interpreter remains executable and the Windows junction fails to resolve; even --reinstall fails. Later comments associate that case with MSIX filesystem virtualization. Here the interpreter itself is absent, Linux is also affected, and --reinstall succeeds.
- astral-sh/uv#19329 — issue, open: `uv python install` publishes managed Python before finalization, allowing `uv python find` to race and observe incomplete installs. Shares managed-installation discovery and completeness concerns, but tracks concurrent discovery before finalization and stale interpreter metadata. The new report removes the executable after installation completes and requires no concurrent process, so it is a separate bug.
- astral-sh/uv#20309 — pull request, open: fix: finalize managed Python installation before atomic rename. Proposed fix for astral-sh/uv#19329 moves finalization before publication. Its diff addresses the installation race, not executable loss from an existing installation. It remains unmerged and is neither a fix for this report nor evidence of a regression.

## Supporting evidence

Source inspection used checkout commit `64e2ac873ff0c589024453666c2b9a5bd89dc250` dated 2026-10-16. This checkout was not built; the behavioral reproduction used the installed uv 0.13.0. The source findings below are consistent with the observed failure, but the reproduction does not trace each internal branch or validate a fix. The relevant crates have been split since some older reports, so their historical source paths differ.

- `crates/uv-python-managed/src/managed.rs:198`: `find_all` enumerates installation directories and constructs installation records without checking interpreter existence.
- `crates/uv-python-commands/src/install.rs:99` and `crates/uv-python-commands/src/install.rs:541`: `InstallRequest::matches_installation` matches the installation key; the ordinary install branch accepts that match as satisfied.
- `crates/uv-python-managed/src/downloads.rs:500`: `fetch_from_url` returns `AlreadyAvailable` when `!reinstall && path.is_dir()`. An existing directory therefore prevents a fresh download even when its interpreter is absent.
- `crates/uv-python-discovery/src/discovery.rs:220`: managed discovery filters version, platform, and requested build, then yields an interpreter path without testing executable existence.
- `crates/uv-python-discovery/src/installation.rs:342`: automatic installation accepts either `AlreadyAvailable` or `Fetched`, finalizes the installation, and queries its interpreter. The download shortcut also affects this recovery path.
- `crates/uv-python-managed/src/managed.rs:518`: canonical executable setup creates aliases or returns immediately when the source and destination are identical; it does not restore the missing underlying interpreter.
- `crates/uv-python-interpreter/src/interpreter.rs:1344`: cached interpreter queries can produce the missing-interpreter error during path canonicalization or metadata access. The reporter's narrower claim that the error arises only when spawning is not necessary to establish the bug.

The diff of astral-sh/uv#22650 adds an executable-existence check to managed installation matching, discovery, and download reuse. It adds separate `python_install_missing_executable` and `python_install_automatic_missing_executable` integration tests. Its author reports that they fail on the earlier main revision and that Linux tests and clippy pass with the fix; those execution results were not independently verified here. There are no review comments or approvals on that PR at inspection time.

## Search scope and ruled-out candidates

The prior related-work investigation searched astral-sh/uv open and closed issues and open, closed, and merged PRs using gh. These GitHub searches were not repeated during behavioral reproduction. Literal searches covered 'Python interpreter not found', 'missing executable', 'python install', 'AlreadyAvailable', 'matches_installation', cpython-3.12.15, and 0.12.23. Separate conceptual searches covered repair, corrupt/broken/incomplete managed installations, deleted interpreters, and redownload/re-download; also used area:uv-python. Checked historical fixes and followed candidate comments and timelines. PR text searches returned no matches even for known candidates, so supplemented them with the latest 100 all-state PRs and direct inspection of linked PRs. Ruled out astral-sh/uv#8298 and merged astral-sh/uv#8310 (Windows free-threaded executable naming), astral-sh/uv#4933 and merged astral-sh/uv#4935 (recreating broken virtual environments), astral-sh/uv#8028 (tool-environment repair), and astral-sh/uv#11355 (a host .venv mounted into Docker). No earlier exact duplicate or previously merged fix for this behavior was established.

The report was decomposed before searching into: successful-but-ineffective explicit installation; failed automatic recovery in run/sync; system-interpreter fallback masking the broken installation; retained installation directory with a missing canonical executable; and the separately proposed directory/key-validation mechanism. Literal error and identifier searches were kept separate from searches for incomplete publication, junction failures, and corruption.

The strongest historical alternatives do not establish a regression:

- astral-sh/uv#8298 concerns Windows Python 3.13 free-threaded distributions containing `python3.13t.exe` instead of `python.exe`. Maintainer comments point to astral-sh/uv#8310, merged on 2024-10-17, which adds executable-name compatibility. This differs from deleting the canonical interpreter from an otherwise installed Python 3.12 distribution.
- astral-sh/uv#4933 was closed by astral-sh/uv#4935, merged on 2024-07-09. That change recreates project/tool virtual environments after their base interpreter is removed; it does not validate or replace the remaining managed interpreter directory.
- astral-sh/uv#8028 requests repair of tool environments after Python upgrades, while astral-sh/uv#11355 was resolved by avoiding a host virtual environment mounted over the container's environment.

PR search results were incomplete: text searches returned empty results while direct issue timelines and PR listings exposed relevant PRs. The handoff therefore does not claim an exhaustive negative search over all historical PRs.

## Next step and verification limits

Review astral-sh/uv#22650 for both explicit repair and automatic recovery, including platform results for deletion of the actual canonical interpreter. Retain the distinction from the junction and publication-race work. The immediate documented workaround is `uv python install --reinstall 3.12`.

The Linux reproduction independently confirms the missing-interpreter failure and the explicit-reinstall workaround. It does not verify Windows, WSL2, uv 0.13.1 or 0.12.23, the checkout binary, `uv sync`, system-interpreter fallback, or uninstall followed by install. No repository build or test suite was run, and no released fix was verified. Related GitHub status and proposed-diff details above are retained from the prior investigation. GitHub and checkout files were not changed; reproduction state and this handoff were written only to temporary directories.
