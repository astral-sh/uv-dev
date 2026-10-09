# `uv python list --only-installed` cause: Querying Python at `D:/path/to/.uv/shims\python3.8.exe` failed with exit status exit code: 1

Issue: astral-sh/uv#22352

Classification: duplicate

## Summary

On Windows 11, interpreter discovery warns that the Python 3.8.20 shim cannot initialize its standard library, then continues listing installations.

Closest match: astral-sh/uv#19199, shipped in uv 0.11.9. The failing Python 3.8 shim predates that fix; no current regression is established.

## Draft response

This appears to match the older Windows launcher failure addressed in astral-sh/uv#19199, shipped in uv 0.11.9. Your failing `python3.8.exe` is dated January 15, before that fix; updating uv does not automatically recreate existing Python launchers.

Please run `uv python install 3.8.20 --reinstall` with the same `UV_PYTHON_INSTALL_DIR` and `UV_PYTHON_BIN_DIR` settings used for these custom directories, then retry `uv python list --only-installed`. If it still fails, please share the output with `-vv` and whether running both the shim and the underlying installation's `python.exe` with `-I -c "import encodings; print(encodings.__file__)"` succeeds. That will help determine whether a current launcher regression remains.

## Classification

The Windows/Python <=3.10 launcher startup failure is already covered by astral-sh/uv#19199. The failing shim predates that fix, and installation code retains an existing same-target launcher unless replacement is requested. Running a newer uv therefore does not establish a regression. The old-launcher explanation needs confirmation by regeneration; a failure with a newly generated launcher should be investigated as a bug.

## Related

- astral-sh/uv#19199 (merged pull request): trampolines: Don't set PYTHONHOME, set __PYVENV_LAUNCHER__ only in venvs. Documents and fixes Windows startup failures on Python 3.10 and older when a launcher outside a virtual environment sets __PYVENV_LAUNCHER__. Merged April 29, 2026 and shipped in uv 0.11.9. The reported failing shim is dated January 15, before the fix; its sys.executable points to the shim while standard-library lookup fails. This closely matches the historical launcher failure, although regeneration is needed to confirm the local cause.

## Report and triggering conditions

The report was opened on October 8, 2026, for Windows 11 x86_64, uv 0.12.23
(46b84fd0b, October 3), and CPython 3.8.20. It reports one failure: interpreter
discovery through the custom `.uv/shims/python3.8.exe` launcher exits with code 1.
There is no separate request about duplicate list entries or Python version management.

- Command: `uv python list --only-installed`.
- Discovery emits `Failed to inspect Python interpreter from search path` and
  `Querying Python ... failed with exit status exit code: 1`.
- Python itself fails during startup with `init_fs_encoding` and
  `ModuleNotFoundError: No module named 'encodings'`.
- The diagnostic has `isolated = 1`, `environment = 0`, and
  `PYTHONHOME = (not set)`. The program name and base executable point into
  `cpython-3.8.20-windows-x86_64-none`, but `sys.executable` points to the shim.
  Its `sys.path` includes relative `.\\DLLs`, `.\\lib`, and the shims directory.
- Listing continues and includes the direct 3.8.20 installation and its minor-version
  junction. This does not independently establish a fresh, uncached successful query
  of that executable.
- The failing launcher is 39,424 bytes and dated January 15, 2026. The working
  3.11–3.13 launchers are 45,568 bytes and dated May or July 2026. File size and
  timestamp support an older launcher hypothesis but do not identify its exact build.

## Supporting evidence

1. astral-sh/uv#19199 explicitly documents that Python 3.10 and earlier can fail
   during startup when `__PYVENV_LAUNCHER__` points to a launcher outside a virtual
   environment and `PYTHONHOME` is unavailable. It changed Python trampolines to
   set `__PYVENV_LAUNCHER__` only when the trampoline is inside a virtual
   environment and removed their `PYTHONHOME` assignment. The diff includes
   regenerated Windows executables. It merged April 29, 2026.
2. The uv 0.11.9 release notes explicitly include astral-sh/uv#19199. The notes
   date the release May 4; GitHub records publication on May 5. The reported
   January shim predates both merge and release.
3. The source at tag `0.12.23`, `crates/uv-trampoline/src/bounce.rs`, still
   contains that fix. In the checkout, lines 138–154 set
   `__PYVENV_LAUNCHER__` only under `is_virtualenv(&executable_name)`;
   lines 199–205 check for a grandparent `pyvenv.cfg`.
4. `crates/uv-python-interpreter/src/interpreter.rs:1262` queries Python with
   `-I -B -c`. This explains the report's isolated startup: Python ignores
   ordinary Python environment configuration under `-I`, so setting
   `PYTHONHOME` externally is not an appropriate repair for this discovery query.
5. `crates/uv-python-commands/src/install.rs:1091` skips replacement when an
   existing launcher targets the same installation, unless `--reinstall` or
   `--force` is supplied. The replacement branch recreates the executable.
   Consequently, the version of uv currently running does not establish the
   version of a previously installed launcher.
6. `crates/uv-python-commands/src/list.rs:148` discovers installed interpreters,
   including PATH entries. `crates/uv-python-discovery/src/discovery.rs:1151`
   warns on interpreter query failures and continues. The report exhibits this
   expected discovery behavior; the substantive failure is Python startup through
   the launcher.
7. Historical context was checked through astral-sh/uv#13531 and its review
   comments, which introduced the earlier launcher `PYTHONHOME` behavior;
   astral-sh/uv#17821, which adjusted inheritance; and astral-sh/uv#19080, whose
   cross-version standard-library contamination prompted astral-sh/uv#19199.
   The latter issue has a different symptom and is not itself the duplicate target.

## Other candidates evaluated

astral-sh/uv#21047 alleges that an invalid pyenv PATH candidate aborts discovery.
A maintainer demonstrated that discovery continues with another usable interpreter.
Here the warning is followed by the interpreter list, so an abort is not the reported problem.

astral-sh/uv#12173, astral-sh/uv#8821, astral-sh/uv#15334, and astral-sh/uv#15517
share the missing-`encodings` error but concern virtual environments built from
symlinked Python installations on Unix-like systems. Maintainers identify
astral-sh/python-build-standalone#380 as the underlying discussion. Its closing
explanation describes a Python 3.11+ path-resolution fix and says Python 3.10 and
earlier used different behavior. These are not evidence that this Windows 3.8
launcher failure is the same bug or a regression of that fix.

astral-sh/uv#19374 and its open proposed fix, astral-sh/uv#20853, concern native
console-script launchers resolving a virtual environment's symlink to the base
interpreter. This report invokes a Python launcher outside a reported virtual
environment and fails to initialize the standard library.

## Verification limits and follow-up

No Windows reproduction or inspection of the reporter's executable has been
performed. An old launcher is the strongest explanation from the supplied paths,
timestamps, historical fix, and replacement logic; it is not independently confirmed.
No evidence shows this worked with a post-fix launcher and then regressed.

Regenerate Python 3.8.20 using the same custom installation and executable-directory
settings, then retry discovery. If it persists, compare isolated `encodings` imports
through the regenerated shim and the direct installation, collect verbose discovery
output, and check whether a `pyvenv.cfg` exists in the shim's grandparent directory.
A failure with a freshly generated launcher should be handled as a possible current
bug rather than closed solely against the historical fix.

## Search coverage

Searched open and closed issues and open, closed, and merged PRs using the exact discovery warning, Querying Python, init_fs_encoding, encodings, python list, and Python 3.8; broadened to Windows shims, trampolines, isolated startup, PYTHONHOME, __PYVENV_LAUNCHER__, and launcher refresh/reinstall. PR keyword searches omitted known matches, so supplemented them with commit history, referenced PRs, reviews, diffs, and release notes. Ruled out astral-sh/uv#21047: maintainers could not reproduce its claimed discovery abort, whereas this report continues listing. Also ruled out astral-sh/uv#12173 and astral-sh/uv#8821 after following their upstream discussion, astral-sh/python-build-standalone#380: those concern virtual environments created through symlinks. astral-sh/uv#19374 and astral-sh/uv#20853 concern console scripts escaping symlinked virtual environments.
