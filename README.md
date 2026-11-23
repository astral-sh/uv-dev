# `uv python upgrade` reports its own bin link as "not managed by uv" when the install path traverses a symlink

Issue: astral-sh/uv#23364

Classification: bug

## Summary

On Linux with uv 0.13.9, a symlinked ancestor of `UV_PYTHON_INSTALL_DIR` can cause `uv python upgrade 3.11` to install CPython 3.11.18 but leave uv's `python3.11` bin link pointing at 3.11.17. The command incorrectly warns that the existing link is not managed by uv. The reporter encountered this with a symlinked home directory on Fedora Atomic and provided a reproduction using temporary directories.

The closest prior report is astral-sh/uv#15669, which had the same warning but no confirmed symlink trigger or linked fix. astral-sh/uv#14247 and astral-sh/uv#14261 cover a separate launcher update bug, while astral-sh/uv#22547 fixes relative install paths rather than symlink aliases.

## Draft response

I reproduced this on Linux with uv 0.13.9: the upgrade installed Python 3.11.18, but uv's `python3.11` link still ran 3.11.17. The ownership check resolves the bin link but compares it against an install path that still contains the symlinked directory. uv should recognize and update its own link without `--force`. For now, set `UV_PYTHON_INSTALL_DIR` to the resolved path of the same directory and run `uv python install 3.11` again. That repaired the link in my test without `--force` or a change to `UV_PYTHON_BIN_DIR`.

## Classification

This is a bug: a valid uv-created link is incorrectly classified as unmanaged and skipped during a patch upgrade. The same symlink-triggered failure also reproduces on uv 0.9.8, which was released before astral-sh/uv#15669 closed. That issue does not establish the symlink trigger or identify a fix, so this is neither a confirmed regression nor a duplicate of an established tracker for this mechanism.

## Related

- astral-sh/uv#15669 (closed): The same Linux upgrade warning appeared for a uv-created `python3.13` link on uv 0.8.15, even after a forced install. The maintainer could not reproduce it later and the reporter said it had stopped; neither a symlinked ancestor nor a fixing commit was identified.
- astral-sh/uv#14247 (closed): An upgrade left Windows shims from `uv python install --default --preview` stale, and a later attempt also printed the ownership warning. The contributor reproduced the default-install upgradeability problem, not the warning, and treated the latter as separate.
- astral-sh/uv#14261 (merged): Closed astral-sh/uv#14247 by making preview default installs without a version argument upgradeable. It did not fix the separate ownership warning or a symlinked Unix install root.
- astral-sh/uv#22547 (merged): Made newly downloaded installation paths absolute to prevent broken minor-version aliases with a relative install directory. It did not resolve symlink components in absolute paths or change bin-link ownership matching.

## Evidence

- The official Linux x86-64 uv 0.13.9 binary reproduced the report in isolated temporary directories: with both paths under a symlinked ancestor, `uv python install 3.11.17` created the bin link and `uv python upgrade 3.11` installed 3.11.18 but warned and left the link on 3.11.17. Using physical paths for both directories repointed the link to 3.11.18 without the ownership warning.
- A symlinked install path with a physical bin path reproduced the warning and stale link. A physical install path with a symlinked bin path upgraded normally. After the failed relink, pointing `UV_PYTHON_INSTALL_DIR` at the same physical directory and running `uv python install 3.11` repaired the link without `--force`.
- `find_matching_bin_link` in `crates/uv-python-commands/src/install.rs:1339` canonicalizes the Unix bin symlink and compares it for path equality against `installation.executable(false)`. `ManagedPythonInstallation::from_path` in `crates/uv-python-managed/src/managed.rs:342` makes the discovered path absolute without resolving its symlinked ancestor. The mismatch reaches the warning-and-skip branch in `crates/uv-python-commands/src/install.rs:1038`, even though both paths refer to the same executable.
- uv 0.9.8, released November 7, 2025, showed the same failure with CPython 3.11.13 to 3.11.14. Its Unix matcher also canonicalizes only the bin link. astral-sh/uv#15669 closed on November 9, 2025, without an identified fix.
- A focused Unix case in `crates/uv/tests/python/python_upgrade.rs` can verify both the absence of the warning and the new link target with a symlinked install ancestor. The existing tests cover normal upgrades and protecting a genuinely unmanaged executable, not this combination. The module already uses the `test-python-managed` feature gate.

## Search scope

Used authenticated `gh` to search open and closed issues and open, closed, and merged PRs in astral-sh/uv. Literal searches covered the executable-exists and not-managed warning fragments, both directory environment variables, uv 0.13.9, and the older issue reference. Symptom searches covered stale or unrepointed bin links, shims, and managed Python patch upgrades. Separate cause and fix searches covered symlinked ancestors and home directories, `/var/home`, canonicalization, ownership matching, relative link targets, and the matcher names. PR search did not surface even a known merged PR by title, so it was supplemented with recent repository PR listings, changelogs, and linked discussions and diffs.

No existing tracker or fix specifically for this symlink-triggered failure surfaced. astral-sh/uv#15955 and astral-sh/uv#15959 were ruled out because they concern version ordering for release candidates after a link was recognized. astral-sh/uv#21639 fixes validation of unmanaged relative symlink targets, not ownership matching. astral-sh/uv#17959 and astral-sh/uv#18459 concern listing links, not repointing them.
