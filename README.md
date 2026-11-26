# `--link-mode=clone` fails with ERROR_MORE_DATA on Windows ReFS Dev Drive

Issue: astral-sh/uv#23408

Classification: bug

## Summary

The reporter's `uv pip install annotated-types==0.8.0 --link-mode=clone` aborts with `ERROR_MORE_DATA` while cloning `annotated_types/test_cases.py` on Windows 11, with the uv cache and virtual environment on the same ReFS Dev Drive. Explicit `--link-mode=copy` succeeds with the same cached wheel and is the reported workaround. The available Linux runner did not exercise Windows block cloning.

## Classification

Reported bug in an explicitly requested per-file clone install. The historical directory-cloning fix is still reflected in the Windows code path, and the report does not identify a last known-good uv release. The failing Windows operation and the significance of the file's extent layout are unconfirmed.

## Reproduction

**Outcome: `not_reproducible` on the available runner.** The x86-64 Linux runner has ext4, not Windows and ReFS. Its successful fallback does not contradict the report; an independent reproduction still requires a Windows ReFS Dev Drive.

### Reported Windows case

- Windows 11 build 26300 (AMD64), PowerShell 7.7.0, ReFS 3.14 Dev Drive with 4096-byte clusters, uv 0.13.9 (`6b2d39dcc`, November 17, 2026, `x86_64-pc-windows-msvc`), and CPython 3.15.2.
- The expected result is a successful install using block cloning. The reporter observed wheel resolution and preparation succeed before clone mode failed with `ERROR_MORE_DATA` (Win32 234, HRESULT `0x800700EA`, signed code `-2147024662`). The failure occurred with fresh and existing caches. The cached `test_cases.py` is 6310 bytes and reportedly occupies two non-contiguous one-cluster extents. A separate native Robocopy control (with and without `/NOCLONE`) worked on the same volume; this does not identify the failing operation in uv's backend.
- To verify, substitute the matching Dev Drive letter and run these commands with the reported uv and Python versions. These commands have not been run on Windows as part of this handoff.

```powershell
$root = Join-Path 'V:\' ('uv-23408-' + [guid]::NewGuid().ToString('N'))
uv venv "$root\clone-env" --python 3.15
uv pip install 'annotated-types==0.8.0' --python "$root\clone-env\Scripts\python.exe" --cache-dir "$root\cache" --link-mode=clone --no-deps -vv
uv venv "$root\copy-env" --python 3.15
uv pip install 'annotated-types==0.8.0' --python "$root\copy-env\Scripts\python.exe" --cache-dir "$root\cache" --link-mode=copy --offline --no-deps -vv
```

### Observed Linux comparison

Used the installed uv 0.13.9 (`x86_64-unknown-linux-gnu`) and CPython 3.12.3. All environments, cache, and temporary files were under a new directory on `/tmp` (ext4). The commands below use `run_uv` for the installed `uv` invoked with an isolated home and environment, `--no-config`, and `--no-python-downloads`. The wheel was fetched from public PyPI with source builds disabled.

```sh
run_uv venv clone-env --python "$python_bin" --cache-dir cache
run_uv pip install 'annotated-types==0.8.0' --python clone-env/bin/python --cache-dir cache --link-mode=clone --no-deps --only-binary :all: -vv
run_uv venv copy-env --python "$python_bin" --cache-dir cache
run_uv pip install 'annotated-types==0.8.0' --python copy-env/bin/python --cache-dir cache --link-mode=copy --offline --no-deps --only-binary :all: -vv
run_uv venv warm-clone-env --python "$python_bin" --cache-dir cache
run_uv pip install 'annotated-types==0.8.0' --python warm-clone-env/bin/python --cache-dir cache --link-mode=clone --offline --no-deps --only-binary :all: -v
```

Both clone attempts logged `Operation not supported (os error 95), falling back` for `annotated_types/test_cases.py`, then installed successfully (exit 0). The installed files shared the cached file's inode, confirming hardlink fallback rather than block cloning. The offline copy control also installed successfully (exit 0) and produced a distinct inode for the same 6310-byte file.

### Existing coverage

No integration test was found for the Windows ReFS per-file `ERROR_MORE_DATA` case in `crates/uv/tests/` or `crates/uv-client/tests/it/`. The closest tests cover different behavior:

- `crates/uv/tests/build/cache_clean.rs:147` (`clean_all_cloned_file`) and `crates/uv/tests/build/cache_clean.rs:186` (`clean_all_cached_clones`) create successful clones on a Unix copy-on-write filesystem and check physical space reported by cache cleanup.
- `crates/uv/tests/pip/pip_sync.rs:216` (`install_copy`) installs MarkupSafe with copy mode and checks that it remains importable after the cache is deleted.
- `crates/uv-fs/src/link.rs:2009` (`test_clone_mode_returns_hardlink_or_copy_when_reflink_unsupported`) checks fallback and file content on a filesystem that does not support reflinks; it skips a filesystem on which the reflink probe succeeds.

## Related

- astral-sh/uv#3547 (closed), “Installing wheel with link-mode=clone fails on windows” — Same command and filesystem, but uv 0.1.42 attempted to clone a directory and got Win32 error 5. The maintainer discussion distinguishes falling back to per-file cloning from falling back to copying.
- astral-sh/uv#3551 (merged), “Clone individual files on windows ReFS” — Closed astral-sh/uv#3547 on May 13, 2024 by cloning files individually instead of attempting to clone directories. It did not address `ERROR_MORE_DATA` from a regular-file clone.
- astral-sh/uv#16153 (open), “Should we use Copy-on-Write for Windows Dev Drives and other ReFS drive?” — Tracks detection, performance, and whether to select copy-on-write on Windows ReFS. It does not track a fatal error when clone mode is selected explicitly.

## Supporting evidence

- **Source:** `crates/uv-install-wheel/src/linker.rs:270` dispatches wheel installation to `link_dir`. In `crates/uv-fs/src/link.rs:328`, whole-directory cloning is macOS-only. Windows walks the tree and delegates individual clone attempts to `reflink_copy::reflink`. `crates/uv-fs/src/link.rs:469` can fall back after an initial clone failure, but propagates an unexpected error after clone mode has succeeded on a file in the walk. This is consistent with the report but does not identify the failing Windows operation.
- **Release check:** uv 0.13.9 shipped November 17, 2026; uv 0.13.10 shipped November 24, 2026. The two tags have identical wheel-linker and filesystem-linker source and both lock `reflink-copy` 0.1.30. The 0.13.10 notes do not identify a fix for this report; the report does not test that release.
- **Next step:** Re-run the clone and offline copy control on Windows with the cache and environments on the same ReFS Dev Drive. If clone fails, retain the failing cached wheel and try cloning that specific file through the Windows reflink backend to identify which operation returns `ERROR_MORE_DATA`.

## Search scope

Searched open and closed issues and open, closed, and merged pull requests in astral-sh/uv. Literal searches covered the HRESULT, signed code, error name, clone error text, package, ReFS, and Dev Drive. Package- and platform-independent searches covered wheel linking, reflinks, copy-on-write, and fallback. Separate cause-oriented searches covered block cloning, extent layout and Windows API identifiers. Checked candidate comments and closing links, pull request listings, linker and dependency history, and the 0.13.9–0.13.10 release comparison. The exact error searches surfaced only the new report.

Excluded astral-sh/uv#17544 (Dev Drive placement guidance, with a report of successful installs using copy mode rather than this fatal error), astral-sh/uv#15084 (APFS fallback and incomplete installs), astral-sh/uv#1444 and astral-sh/uv#1773 (cross-volume copy fallback), astral-sh/uv#7620 (Linux directory-clone fallback), and astral-sh/uv#23029 (an opt-in mode for cross-device fallback, not a fix for a file clone failure on the same volume).
