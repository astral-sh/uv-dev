# `uv python upgrade` reports its own bin link as "not managed by uv" when the install path traverses a symlink

Issue: astral-sh/uv#23364

Classification: bug

## Summary

Reproduced on Linux x86_64 with uv 0.13.9. When `UV_PYTHON_INSTALL_DIR` traverses a symlink, `uv python upgrade 3.11` can install CPython 3.11.18 but leave uv's `python3.11` bin link pointing to 3.11.17. The command exits successfully while warning that the link is not managed by uv. The reporter encountered this with a symlinked home directory on Fedora Atomic; the same behavior occurs with an ordinary directory symlink on Ubuntu.

**Verified workaround:** Set `UV_PYTHON_INSTALL_DIR` to the physical path of the same installation directory and run `uv python install 3.11` again. This repaired the link without `--force` or a change to `UV_PYTHON_BIN_DIR`.

## Classification

This is a bug: the link was created by uv and still points to a uv-managed interpreter, so the upgrade should repoint it without `--force`. No regression window is established. Prior triage also reproduced the warning and stale link on uv 0.9.8 with CPython 3.11.13 to 3.11.14. The similar warning in astral-sh/uv#15669 does not establish a duplicate: that report did not confirm the symlink trigger or identify a fix.

## Reproduction

**Outcome: reproducible.** Tested with the installed `uv 0.13.9 (x86_64-unknown-linux-gnu)` on Ubuntu 24.04.5 LTS, kernel `7.0.0-1017-azure`, in an isolated temporary directory with no user configuration. The report used Homebrew uv 0.13.9 on Fedora Atomic and listed Python 3.14.10; the actual reproduction installs and upgrades managed CPython in the 3.11 series and does not require Python 3.14.10.

```sh
(
  scratch=$(mktemp -d "${TMPDIR:-/tmp}/uv-23364.XXXXXX")
  mkdir -p "$scratch/real" "$scratch/home" "$scratch/tmp"
  ln -s "$scratch/real" "$scratch/alias"
  cd "$scratch" || exit 1
  export HOME="$scratch/home" TMPDIR="$scratch/tmp" UV_NO_CONFIG=1
  export XDG_DATA_HOME="$scratch/data" XDG_CONFIG_HOME="$scratch/config"
  export XDG_CACHE_HOME="$scratch/xdg-cache" UV_CACHE_DIR="$scratch/cache"
  export UV_PYTHON_INSTALL_DIR="$scratch/alias/py" UV_PYTHON_BIN_DIR="$scratch/alias/bin"
  uv python install 3.11.17
  readlink "$UV_PYTHON_BIN_DIR/python3.11"
  "$UV_PYTHON_BIN_DIR/python3.11" --version
  uv python upgrade 3.11
  printf 'upgrade exit: %s\n' "$?"
  readlink "$UV_PYTHON_BIN_DIR/python3.11"
  "$UV_PYTHON_BIN_DIR/python3.11" --version
)
```

**Expected:** The upgrade installs 3.11.18 and repoints uv's `python3.11` link to the upgraded interpreter without an ownership warning.

**Observed:** The initial install creates the link and it runs `Python 3.11.17`. The upgrade exits `0` and installs a working CPython 3.11.18, but warns:

```text
warning: Executable already exists at `…/alias/bin/python3.11` but is not managed by uv; use `uv python install 3.11 --force` to replace it
```

`readlink` shows the same 3.11.17 target before and after the upgrade, and invoking the link still prints `Python 3.11.17`.

| Install path | Bin path | Ownership warning | Python run through the bin link after upgrade |
| --- | --- | --- | --- |
| Symlinked ancestor | Symlinked ancestor | Yes | 3.11.17 |
| Physical | Physical | No | 3.11.18 |
| Symlinked ancestor | Physical | Yes | 3.11.17 |
| Physical | Symlinked ancestor | No | 3.11.18 |

**Implementation evidence:** The resolved bin link and the old interpreter under the aliased install path refer to the same inode. This is consistent with the Unix matcher in `crates/uv-python-commands/src/install.rs:1339`, which canonicalizes the bin link and compares it by path equality with `installation.executable(false)`. The discovered install path is made absolute without resolving symlink ancestors in `crates/uv-python-managed/src/managed.rs:342`.

**Existing tests:** `python_upgrade` in `crates/uv/tests/python/python_upgrade.rs:14` checks successful ordinary patch upgrades, and `python_upgrade_force_install` in `crates/uv/tests/python/python_upgrade.rs:520` checks that uv warns about an unmanaged executable explicitly created in the bin directory. Neither test checks upgrading uv's own link through a symlinked install ancestor. The module is gated by `test-python-managed`.

## Related

- astral-sh/uv#15669 (closed): The same Linux upgrade warning appeared for a uv-created `python3.13` link on uv 0.8.15, even after a forced install. The maintainer could not reproduce it later and the reporter said it had stopped; neither a symlinked ancestor nor a fixing commit was identified.
- astral-sh/uv#14247 (closed): An upgrade left Windows shims from `uv python install --default --preview` stale, and a later attempt also printed the ownership warning. The contributor reproduced the default-install upgradeability problem, not the warning, and treated the latter as separate.
- astral-sh/uv#14261 (merged): Closed astral-sh/uv#14247 by making preview default installs without a version argument upgradeable. It did not fix the separate ownership warning or a symlinked Unix install root.
- astral-sh/uv#22547 (merged): Made newly downloaded installation paths absolute to prevent broken minor-version aliases with a relative install directory. It did not resolve symlink components in absolute paths or change bin-link ownership matching.

- Prior triage searched open and closed issues and pull requests using the warning, directory settings, upgrade symptoms, and symlink matching terms. No tracker or fix specifically for this failure surfaced. astral-sh/uv#15955 and astral-sh/uv#15959 concern release-candidate ordering after a link was recognized; astral-sh/uv#21639 concerns validation of unmanaged relative symlink targets; astral-sh/uv#17959 and astral-sh/uv#18459 concern listing links, not repointing them.
