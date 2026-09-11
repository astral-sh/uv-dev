# Regression in uv 0.12.11 with long temp paths on Windows

Issue: astral-sh/uv#21611

Classification: bug

## Summary

The report describes a controlled Windows 11 comparison for
`uv pip install jupyterlab-widgets==3.0.16`: with `LongPathsEnabled=0`, uv 0.12.10
succeeds and uv 0.12.11 repeatedly fails while persisting a temporary file to a deeply nested
path under `site-packages`, ending with OS error 3. Both versions reportedly succeed when
`LongPathsEnabled=1`.

The behavior could not be meaningfully exercised on the available Linux runner because the
Windows registry setting and Win32 path APIs are essential to the report. A Linux control with
fresh environments and paths longer than the reported path succeeded in both versions, but that
result neither confirms nor contradicts the Windows behavior.

The report shows that uv loaded both a workspace configuration and a user `uv.toml`, but does not
include their contents. This matters because astral-sh/uv#21468 changed the copy path used when
merging a wheel into `site-packages`, while Windows normally defaults to hard links. The active
link mode, cache location and volume, and whether the displayed project path is exact are needed
to reconstruct the failing code path reliably.

## Classification

This remains classified as a **bug** because the reported Windows version matrix is consistent
with an installation regression, and no existing issue or pull request tracks this exact failure.
The classification is provisional until the native-Windows reproduction is independently
observed. It is not a duplicate of astral-sh/uv#16877, which concerned symlink mode and different
Windows errors.

## Reproduction

Outcome: **needs more information**.

The available host was Linux x86_64, with uv 0.12.13 installed on `PATH`; it cannot toggle or
observe Windows `LongPathsEnabled`. To provide a controlled comparison, uv 0.12.10 and 0.12.11
were installed as isolated tools under `/tmp`, configuration discovery was disabled, and each was
run against public PyPI with a fresh managed Python 3.10.21 environment. Both default and explicit
copy modes were tested. Each `(version, mode)` combination used a separate fresh `$case` directory;
the command template was:

```console
export UV_NO_CONFIG=1
export UV_CACHE_DIR="$case/cache"
export UV_TOOL_DIR="$case/tools"
export UV_TOOL_BIN_DIR="$case/tool-bin"
export UV_PYTHON_INSTALL_DIR="$case/python"

cd "$case/project-with-a-100-character-path-component"
uvx --from "uv==$version" uv venv --python 3.10
uvx --from "uv==$version" uv pip install $mode_args jupyterlab-widgets==3.0.16
```

The combinations were `version=0.12.10` and `version=0.12.11`, each with `mode_args` empty and
with `mode_args="--link-mode copy"`.

All four fresh-environment cases succeeded. The reported JavaScript file existed after each
install at an absolute path 308–311 characters long. This is only a Unix control: Unix does not
apply the Win32 `MAX_PATH` behavior, so it is not evidence that the report is incorrect.

The release notes and astral-sh/uv#21468 were inspected. That pull request merged between 0.12.10
and 0.12.11 and replaced a per-file temporary directory plus `fs_err::rename` with
`copy_atomic_sync`, which creates an adjacent `NamedTempFile` and persists it to the destination.
The latter operation emits the warning and final error in the report. This is an evidence-backed
change to test, not a confirmed root cause. astral-sh/uv#21478 also changed adjacent temporary
paths for overwriting hard links, symlinks, and reflinks in 0.12.11, but the reported
`Failed to persist temporary file` wording matches the copied-file path.

Existing tests do not cover the reported matrix. In `crates/uv-fs/src/link.rs`,
`test_merge_overwrites_existing_files` exercises a short-path copy merge, and
`test_copy_merge_replaces_symlink_and_preserves_permissions` is Unix-only. The
`crates/uv/tests/it/ecosystem.rs::jupyterlab` test only locks the ecosystem fixture; it does not
install `jupyterlab-widgets` or exercise Windows long paths. No relevant integration test was
found under `crates/uv-client/tests/it/`.

An independent reproduction needs a native Windows 11 x86_64 host and the following additions to
the reported matrix:

- Confirm whether `C:\projects\foo` is the exact project root; the displayed failing target is
  243 characters, so any anonymized prefix affects the path boundary.
- Provide the relevant redacted contents of the loaded workspace `pyproject.toml`, user `uv.toml`,
  and `UV_*` environment settings, especially `link-mode`, `cache-dir`, and index configuration.
- State whether the cache and virtual environment are on the same Windows volume.
- Repeat 0.12.10 and 0.12.11 with `LongPathsEnabled=0`, a fresh cache and virtual environment,
  `--no-config`, public PyPI, and explicit `--link-mode hardlink` and `--link-mode copy`. This will
  distinguish default hard-link behavior from the merged-copy path changed by astral-sh/uv#21468.

## Related

- astral-sh/uv#21468 — merged pull request, “Avoid per-file temporary directories for merged
  copies.” It is the strongest release-specific lead and introduced the persistence operation
  named in the failure, but its Windows causality has not been observed here.
- astral-sh/uv#21478 — merged pull request, “Create atomic replacement links without temporary
  directories.” It also landed in 0.12.11 and changed overwrite paths for non-copy link modes, but
  does not directly match the reported copied-file error wording.
- astral-sh/uv#16877 — closed issue involving the same package and deeply nested Windows paths,
  but in symlink mode with OS errors 1314 and 87 rather than copied-file persistence and OS error 3.
- astral-sh/uv#16894 — merged pull request that added uv's Windows long-path-aware manifest while
  documenting that Windows must also have `LongPathsEnabled=1`.
