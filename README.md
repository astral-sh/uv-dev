# `--link-mode=clone` fails with ERROR_MORE_DATA on Windows ReFS Dev Drive

Issue: astral-sh/uv#23408

Classification: bug

## Summary

On uv 0.13.9, `uv pip install annotated-types==0.8.0 --link-mode=clone` fails while installing `annotated_types/test_cases.py` on a Windows 11 ReFS Dev Drive. Both the cache and virtual environment are on the same volume. The closest historical report and fix, astral-sh/uv#3547 and astral-sh/uv#3551, addressed directory cloning. The open ReFS discussion in astral-sh/uv#16153 concerns selecting copy-on-write, not this per-file failure.

## Draft response

Thanks for the repro. On Windows, uv already clones wheel files individually, so this is different from the directory-cloning failure fixed in astral-sh/uv#3551. `--link-mode=copy` is a workaround. The next step is to try cloning that same cached file directly through the Windows reflink backend and identify which operation returns `ERROR_MORE_DATA`. The Robocopy test shows block cloning can work on the volume, but the cause of this failure is still unknown.

## Classification

The explicitly requested, supported per-file clone mode aborts a same-volume wheel install, while copying the same cached wheel succeeds. No earlier fix or active thread was found for this error on an individual file. The historical directory-cloning fix is still reflected in the Windows code path, so the report does not establish a regression of that bug. The operation that returns the Windows error, and whether allocation layout contributes, are unconfirmed.

## Related

- astral-sh/uv#3547 (closed), “Installing wheel with link-mode=clone fails on windows” — Same command and filesystem, but uv 0.1.42 attempted to clone a directory and got Win32 error 5. The maintainer discussion distinguishes falling back to per-file cloning from falling back to copying.
- astral-sh/uv#3551 (merged), “Clone individual files on windows ReFS” — Closed astral-sh/uv#3547 on May 13, 2024 by cloning files individually instead of attempting to clone directories. It did not address `ERROR_MORE_DATA` from a regular-file clone.
- astral-sh/uv#16153 (open), “Should we use Copy-on-Write for Windows Dev Drives and other ReFS drive?” — Tracks detection, performance, and whether to select copy-on-write on Windows ReFS. It does not track a fatal error when clone mode is selected explicitly.

## Supporting evidence

- **Reported reproduction:** Windows 11 build 26300, ReFS 3.14, 4096-byte clusters, and CPython 3.15.2. With a fresh or existing cache, dependency resolution and wheel preparation succeed before the file clone aborts with `ERROR_MORE_DATA` (Win32 234, HRESULT `0x800700EA`, signed code `-2147024662`). Explicit copy mode succeeds offline with the same cached wheel. The affected file is 6310 bytes and spans two non-contiguous one-cluster extents; a separate Robocopy test verified native block cloning on the volume. No independent Windows reproduction was performed for this handoff.
- **Source:** `crates/uv-install-wheel/src/linker.rs:253` dispatches wheel installation to `link_dir`. In `crates/uv-fs/src/link.rs:328`, whole-directory cloning is macOS-only. Windows walks the tree and delegates individual clone attempts to `reflink_copy::reflink`. `crates/uv-fs/src/link.rs:469` can fall back after an initial clone failure, but propagates an unexpected error after clone mode has succeeded on a file in the walk. That is consistent with a fatal per-file error and does not identify the Windows operation that failed.
- **Release check:** uv 0.13.9 shipped November 17, 2026; uv 0.13.10 shipped November 24, 2026. The two tags have identical wheel-linker and filesystem-linker source and both lock `reflink-copy` 0.1.30. The 0.13.10 notes do not identify a fix for this report; the report does not test that release.

## Search scope

Searched open and closed issues and open, closed, and merged pull requests in astral-sh/uv. Literal searches covered the HRESULT, signed code, error name, clone error text, package, ReFS, and Dev Drive. Package- and platform-independent searches covered wheel linking, reflinks, copy-on-write, and fallback. Separate cause-oriented searches covered block cloning, extent layout and Windows API identifiers. Checked candidate comments and closing links, pull request listings, linker and dependency history, and the 0.13.9–0.13.10 release comparison. The exact error searches surfaced only the new report.

Excluded astral-sh/uv#17544 (Dev Drive placement guidance, with a report of successful installs using copy mode rather than this fatal error), astral-sh/uv#15084 (APFS fallback and incomplete installs), astral-sh/uv#1444 and astral-sh/uv#1773 (cross-volume copy fallback), astral-sh/uv#7620 (Linux directory-clone fallback), and astral-sh/uv#23029 (an opt-in mode for cross-device fallback, not a fix for a file clone failure on the same volume).
