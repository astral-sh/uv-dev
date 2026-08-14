# `uv python install` ignores `--default` when older version is specified

Issue: astral-sh/uv#21125

Classification: bug

## Summary

The reported behavior is reproducible. After `uv python install --default`
installed CPython 3.14.7, running `uv python install --default 3.14.6`
installed 3.14.6 successfully, but the `python` executable continued to run
3.14.7. The report is for uv 0.12.3 on macOS 15 x86_64; the reproduction used
the installed uv 0.12.4 executable on Linux x86_64.

The discussion clarifies that replacing the default with an older patch is not
the intended behavior. A maintainer stated that `--default` applies at the
minor-version level and that uv likely should not replace the default with an
older patch, while noting that the implementation details would need to be
rechecked. The actionable correctness problem is the lack of feedback: the
command succeeds and reports the requested 3.14.6 executables even though they
continue to run 3.14.7. The maintainer agreed that warning in this situation is
reasonable.

The reproduced links confirm the implementation mechanism. The executable in
the bin directory points through a `PythonMinorVersionLink`. After link
creation, `uv python install` selects the highest installed patch for each
minor-version key and makes that intermediary link target the highest patch.
With both 3.14.6 and 3.14.7 installed, the unversioned executable therefore
continues to resolve through the 3.14 link to 3.14.7. This minor-version
indirection was introduced for transparent upgrades by astral-sh/uv#13954.

The reporter clarified that selecting an older patch was only for testing a
Zed issue when the Python version changed, not a production requirement, and
that using a different minor version would have served the same purpose. This
reduces the priority of adding patch-level default selection and leaves the
missing warning or misleading success report as the concrete issue.

No existing issue or pull request was found that tracks this stable
older-patch feedback case. astral-sh/uv#15237 has the closest user-visible
symptom, but it concerns a prerelease minor-version request for which the
unversioned links were not created at all. That prerelease mechanism was fixed
by astral-sh/uv#16706 and is not the same problem.

## Reproduction

Outcome: reproducible with uv 0.12.4 (`x86_64-unknown-linux-gnu`) on Linux
x86_64. The report's exact command sequence was run with the managed Python
install directory, executable directory, and cache isolated under `/tmp` and
configuration discovery disabled:

```console
$ export UV_CACHE_DIR=/tmp/uv-issue-21125/cache
$ export UV_PYTHON_INSTALL_DIR=/tmp/uv-issue-21125/python
$ export UV_PYTHON_BIN_DIR=/tmp/uv-issue-21125/bin
$ uv --no-config python install --default
Installed Python 3.14.7 ...
 + cpython-3.14.7-linux-x86_64-gnu (python, python3, python3.14)
$ /tmp/uv-issue-21125/bin/python --version
Python 3.14.7
$ uv --no-config python install --default 3.14.6
Installed Python 3.14.6 ...
 + cpython-3.14.6-linux-x86_64-gnu (python, python3, python3.14)
$ /tmp/uv-issue-21125/bin/python --version
Python 3.14.7
$ readlink -f /tmp/uv-issue-21125/bin/python
/tmp/uv-issue-21125/python/cpython-3.14.7-linux-x86_64-gnu/bin/python3.14
```

The second install succeeded and reported the requested 3.14.6 executables,
but `python` still ran 3.14.7. Inspection also showed `python`, `python3`, and
`python3.14` linked through the shared `cpython-3.14-linux-x86_64-gnu`
directory, which resolved to the higher installed patch.

No existing integration test covers this exact newer-then-older patch sequence
with `--default`. In `crates/uv/tests/python/python_install.rs`,
`python_install_default` verifies creation and replacement of the default links
for the latest patch and for a different minor version, but never requests an
older patch of the same minor. `install_transparent_patch_upgrade_uv_venv`
separately verifies that the shared minor-version link continues to select the
highest patch for a virtual environment; it does not exercise the command
output or warning behavior for an explicit older-patch `--default` request.

## Maintainer discussion

A maintainer indicated that `--default` is intended to operate at the minor
version level, so retaining the highest installed 3.14 patch is expected. They
considered a warning reasonable when an explicit older patch is installed but
cannot become the effective default. The reporter confirmed there is no
specific need for patch-level default selection; the older patch was chosen to
test a Zed issue, and changing minor versions would also work.

## Classification

Retain the bug classification, but narrow it to misleading command behavior
rather than missing patch-level selection. The reported command completes
successfully and lists `python`, `python3`, and `python3.14` for 3.14.6 even
though those executables resolve to 3.14.7, with no warning that defaults are
minor-version scoped. Repository evidence explains the behavior:

- astral-sh/uv#8650 introduced `--default` and replacement of uv-managed
  `python` and `python3` executables, before transparent patch upgrades added
  the shared minor-version intermediary.
- The current `create_bin_links` path treats a `--default` install as
  upgradeable and links through `PythonMinorVersionLink`.
- `highest_installations_by_minor_version_key` then selects 3.14.7 over 3.14.6
  and `ensure_minor_version_link` points the shared 3.14 intermediary at that
  highest installation.

This explains the absence of a conflict warning: uv recognizes the existing
executables as managed and follows its normal link path, but the shared
minor-version intermediary remains on the highest patch. The maintainer's
comment establishes that retaining 3.14.7 is likely intended; the incorrect
part is silently implying that 3.14.6 became the default. No open issue or pull
request already tracks this exact feedback problem, so it is not a duplicate.

## Related

- astral-sh/uv#8650 — merged pull request, “Add `uv python install
  --default`.” It introduced the flag. Its maintainer discussion explicitly
  establishes that `--default` should override uv-managed default executables.
  It also records the important distinction that an ordinary downgrade of the
  minor-version executable requires `--force`.
- astral-sh/uv#13954 — merged pull request, “Support transparent Python patch
  version upgrades.” It introduced the minor-version symlink or junction used
  by upgradeable bin installations. The pull request defines
  `PythonMinorVersionLink` as pointing to the highest installed patch for its
  minor-version key, directly matching the mechanism visible in current source.
- astral-sh/uv#15237 — open issue, “`uv python install --default 3.14` doesn't
  make 3.14 my global python.” It is the closest symptom-level report, but is
  adjacent rather than duplicate: its trigger was a prerelease minor request
  and its output omitted `python` and `python3` entirely. The corresponding
  prerelease handling was fixed later by astral-sh/uv#16706, whereas
  astral-sh/uv#21125 creates managed links that resolve to a newer stable patch.

## Search evidence

Literal searches covered `uv python install --default`, the exact versions
3.14.6 and 3.14.7, `.local/bin`, symlinks, and older-version wording.
Conceptual searches covered global or default Python selection, unversioned
executables, managed-link replacement, patch downgrades, latest-patch
preference, minor-version links, and transparent upgrades. Fix-oriented
searches covered closed issues and merged pull requests for executable
repointing and patch-upgrade behavior, including astral-sh/uv#8733,
astral-sh/uv#14247, and astral-sh/uv#14261 in addition to the closest items
above. astral-sh/uv#16696 was also inspected and ruled out as a duplicate
because it is the fixed prerelease-link creation case.
