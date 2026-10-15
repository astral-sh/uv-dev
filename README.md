# license-file globs find non-license files and complain about encoding

Issue: astral-sh/uv#22579

## Summary

Reproduced with the installed uv 0.13.0 frontend, the resolved uv_build 0.13.1 backend, and Python 3.12.3 on Linux x86_64. With `license-files = ["LICENSE.txt", "src/zzz/_vendor/**/LICENSE.txt"]`, `uv build --wheel` rejects an unrelated `src/zzz/_vendor/thing.bin` containing byte `0xff` as a non-UTF-8 license file. A real cached Python bytecode file also triggers the reported error.

Replacing the recursive pattern with the exact vendored license path succeeds, retains the binary unchanged beside the package code, and copies only the actual licenses into `.dist-info/licenses`. A separate control with UTF-8 content in `thing.bin` builds successfully but incorrectly lists that file in `License-File` metadata and copies it into `.dist-info/licenses`. This confirms that the problem includes selection of non-license files, beyond the encoding error.

No duplicate was identified in the existing related-issue research. astral-sh/uv#16693 and astral-sh/uv#16697 cover unmatched license globs; astral-sh/uv#16751 and astral-sh/uv#18772 concern related directory-matching semantics.

## Classification

Bug: the recursive license pattern selects regular files whose names do not match it. The runtime failures and wheel inspection substantiate this classification. UTF-8 validation of an actual license file is expected; applying it to unrelated package data or cached bytecode is the erroneous behavior.

Including package data beside source code, and retaining a vendored license there while also copying it into `.dist-info/licenses`, are expected behavior. No release regression or previously fixed version was established.

## Related

- astral-sh/uv#16693 — uv build does not warn or fail when license files do not exist (closed issue). Related license-glob validation history: unmatched patterns silently produced empty license directories. This report instead selects an existing file that does not match the pattern, so it is a different correctness problem.
- astral-sh/uv#16697 — Error when a `project.license-files` glob matches nothing (merged pull request). Fixed astral-sh/uv#16693 on 2025-11-14 by checking that each license glob matches a file. Its diff retains the existing match_path-based file collection; the exact matchers only track unmatched patterns. It does not fix the false-positive selection reported here.
- astral-sh/uv#16751 — Build backend: `source-include` silently ignores directory without a glob pattern (open issue). Discusses the same GlobDirFilter and the distinction between directory traversal and inclusion. Its requested behavior concerns including children of explicitly selected directories, whereas this report concerns incorrectly treating nonmatching regular files as licenses.
- astral-sh/uv#18772 — Don't silently ignore directories in source-include (open pull request). Open work for astral-sh/uv#16751 changes the shared directory matcher to include descendants of matched directories. The inspected diff retains match_path's directory-prefix acceptance and does not address this license-file false positive; relevant for coordinating changes to the matcher.

## Report and scope

The reporter encountered this while experimenting with migrating pip to uv_build. Their minimal fixture contains a top-level `LICENSE.txt`, `src/zzz/__init__.py`, a vendored `LICENSE.txt`, and a one-byte binary resource. They expect the recursive glob to select only files named `LICENSE.txt`, leaving binary resources intact.

Reported environment: Linux Mint 22.3, kernel 6.8.0-146-generic x86_64, Python 3.12.3, and uv 0.11.25. The separate backend requirement is `uv_build>=0.13.1,<0.14`; the reporter did not supply the exact resolved backend version. The report also describes errors for cached bytecode and says changing `tool.uv.build-backend.data` does not help, without supplying that configuration.

## Reproduction

**Outcome: reproducible.**

### Environment and isolation

- Frontend: installed `/opt/hostedtoolcache/uv/0.13.0/x86_64/uv`, reporting `uv 0.13.0 (x86_64-unknown-linux-gnu)`.
- Backend: `uv_build==0.13.1`, selected from public PyPI by the unchanged reported requirement. Verbose output confirms this version was installed in the isolated PEP 517 build environment; the frontend's bundled direct-build path was not used because 0.13.0 does not satisfy the requirement.
- Interpreter: `/usr/bin/python`, Python 3.12.3.
- Platform: Linux 6.17.0-1022-azure x86_64.
- Fixtures, caches, build environments, logs, and wheels are under `/tmp/uv-22579-ufvt2561`. Subprocesses used an explicit environment containing only PATH, locale, and temporary-directory/uv isolation settings. Configuration discovery, keyring access, and Python downloads were disabled.
- The reported uv 0.11.25 frontend and Linux Mint installation were not tested. The observed failure uses a backend satisfying the report's exact requirement and the same Python version.

### Minimal configuration and commands

The following reconstructs the tested binary-resource fixture in a new temporary directory:

```sh
repro_dir=$(mktemp -d /tmp/uv-22579.XXXXXX)
cd "$repro_dir"
mkdir -p src/zzz/_vendor tmp
printf '' > src/zzz/__init__.py
printf 'MIT license fixture\n' > LICENSE.txt
printf 'Vendored license fixture\n' > src/zzz/_vendor/LICENSE.txt
python -c 'from pathlib import Path; Path("src/zzz/_vendor/thing.bin").write_bytes(bytes([255]))'
cat > pyproject.toml <<'PYPROJECT'
[project]
version = "0.1.0"
name = "zzz"
description = "zzz"
license = "MIT"
license-files = ["LICENSE.txt", "src/zzz/_vendor/**/LICENSE.txt"]

[build-system]
requires = ["uv_build>=0.13.1,<0.14"]
build-backend = "uv_build"
PYPROJECT

env -i PATH="$PATH" LANG=C.UTF-8 TMPDIR="$repro_dir/tmp" \
  UV_CACHE_DIR="$repro_dir/cache" \
  UV_PYTHON_INSTALL_DIR="$repro_dir/python" \
  UV_KEYRING_PROVIDER=disabled UV_NO_PROGRESS=1 \
  uv --no-config build --wheel --python /usr/bin/python \
  --no-python-downloads --default-index https://pypi.org/simple -v
```

The build exits **2**, with the backend exiting **1**, and reports:

```text
Error: Invalid project metadata

Caused by:
    License file `src/zzz/_vendor/thing.bin` must be UTF-8 encoded
```

The backend version selected during this run was 0.13.1. To reproduce with that version after newer releases become available, pin the build requirement to `uv_build==0.13.1`. The recorded run used the original range.

### Targeted controls

Each control used a separate copy of the original fixture and the same build command without `-v`.

1. **Exact license path:** Change only `src/zzz/_vendor/**/LICENSE.txt` to `src/zzz/_vendor/LICENSE.txt`. The build exits **0**. Wheel inspection confirms `zzz/_vendor/thing.bin` still contains exactly byte `0xff`, and the vendored license exists both beside the package code and under `.dist-info/licenses/src/zzz/_vendor/LICENSE.txt`. `METADATA` has only the two real `License-File` entries; no binary is copied into the license directory.
2. **Real cached bytecode:** Keep the recursive pattern, remove `thing.bin`, write `answer = 42` to `src/zzz/_vendor/helper.py`, then run `python -m py_compile src/zzz/_vendor/helper.py`. The build exits **2** with `License file src/zzz/_vendor/__pycache__/helper.cpython-312.pyc must be UTF-8 encoded` (the diagnostic surrounds the path with backticks). This reproduces the cache-related claim using Python-generated bytecode.
3. **UTF-8 non-license content:** Keep the recursive pattern and replace the contents of `thing.bin` with `ordinary non-license data` plus a newline. The build exits **0**, but `METADATA` incorrectly contains `License-File: src/zzz/_vendor/thing.bin`, and the wheel contains both `zzz/_vendor/thing.bin` and `zzz-0.1.0.dist-info/licenses/src/zzz/_vendor/thing.bin`.

Logs: `/tmp/uv-22579-ufvt2561/reported-range.log`, `exact-license-path.log`, `cached-bytecode.log`, and `utf8-non-license.log`. Successful wheels remain in each control project's `dist` directory. No `tool.uv.build-backend.data` variant was tested; the exact-path control already demonstrates the documented default inclusion of package data.

### Existing test coverage

Searched `crates/uv/tests/` and `crates/uv-client/tests/it/` for license-file/glob coverage and read the relevant test setup and assertions. In `crates/uv/tests/build/build_backend.rs`:

- `license_glob_without_matches_errors` (line 1009) uses `["abc", "LICENSE.txt"]`, creates `LICENSE.txt`, and expects an error for the unmatched `abc` pattern.
- `license_file_must_be_utf8` (line 1051) explicitly selects `LICENSE.bin`, writes byte `0xff`, and expects the UTF-8 error. It covers intentional selection of a non-UTF-8 license, not accidental selection of an unrelated file.
- `symlinked_file` (line 1093, Unix-only) explicitly selects a symlinked `LICENSE`, builds distributions, installs the wheel, and verifies the installed license is a regular file with the target's text.

These tests do not cover recursive license globs alongside unrelated binary resources or cached bytecode. No focused coverage for that combination was found. Tests were inspected, not run or modified. The build_backend module is gated by `test-python` in `crates/uv/tests/build/main.rs`.

## Supporting implementation evidence

Inspected checkout commit: `91122e71cc5a9d213225c33299c4914e4a08e0d1`. Runtime observations above are independent of this source inspection; no binary was built from the checkout.

- `crates/uv-globfilter/src/glob_dir_filter.rs:78`: `match_path` returns `match_directory(path) || glob_set.is_match(path)`. The directory test accepts prefixes that could have matching descendants and does not inspect filesystem type.
- `crates/uv-build-backend/src/metadata.rs:786`: license discovery traverses using the directory filter, then accepts regular files or symlinks using `match_path`. The exact per-pattern matchers only record whether each pattern matched anything; they do not gate insertion into `license_files`. Every collected file is then checked for UTF-8. This is consistent with the observed false-positive selection.
- `crates/uv-build-backend/src/wheel.rs:544`: `wheel_subdir_from_globs` likewise uses `match_path` to select license entries. The UTF-8 control confirms extra files appear in the produced wheel's license directory as well as its metadata.
- `docs/concepts/build-backend.md:221`: package modules include data beside their code; license files are additionally copied into `.dist-info`. Separate configured data directories are copied into `.data`. The successful exact-path control confirms the relevant package-data behavior.

The previously inspected diff of astral-sh/uv#16697 added per-pattern missing-match checks while retaining the existing file collection. It does not establish a regression boundary for this report. The related directory-inclusion work in astral-sh/uv#18772 likewise does not establish a fix for this failure.

## Suggested follow-up

Review full glob matching for regular-file selection in both license metadata collection and wheel copying, while retaining prefix checks for directory traversal. Coordinate shared matcher changes with astral-sh/uv#18772.

Add a focused integration regression in the existing build_backend module using explicit fixture setup and `uv_snapshot!`, with appropriate feature gates. Verify that a recursive license glob builds successfully with an unrelated binary, retains that binary in the package, and excludes it from both license metadata and `.dist-info/licenses`, while retaining the actual vendored license. Cached bytecode is a separately observed failure worth covering.

A tested workaround is to enumerate the actual license paths explicitly, including nested vendored licenses as needed. Binary resources do not need re-encoding or relocation.

## Search coverage and limits

Prior issue-context research searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separated recursive license selection/UTF-8 rejection from package-data placement and license copying. Literal queries included 'must be UTF-8 encoded', 'license-files', 'license glob', 'non-license', and '0.13.1'; conceptual queries covered recursive matching, unrelated/extra files, vendored licenses, package data, and area:build-backend. Separate mechanism searches used GlobDirFilter and match_directory. Inspected candidate comments, timelines, PR reviews, and diffs. PR keyword searches returned no results despite timeline-linked PRs being retrievable, limiting search completeness. Followed astral-sh/uv#16693 to merged astral-sh/uv#16697 and astral-sh/uv#16751 to open astral-sh/uv#18772. Ruled out astral-sh/uv#15040 (default license discovery), astral-sh/uv#13280 (escaping unsupported glob characters), and astral-sh/uv#15655 (wrong backend); astral-sh/uv#16596 clarifies package-data placement but does not track this failure.

No existing canonical discussion of the same failure was identified. Because keyword PR searches did not surface even the independently verified historical PR, absence of further matches should not be treated as exhaustive proof.

The related-issue search findings above were retained from the existing context; GitHub searches were not repeated for the runtime reproduction. No upstream fix or historical version comparison was tested.

## Handoff status

Runtime reproduction and wheel inspection are complete. Only this issue-context README was updated outside the temporary reproduction directory. No repository files or GitHub state were changed, and no response was posted.
