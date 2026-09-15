# 0.12.14: `uv pip install --target=.` fails for every wheel with "The wheel is invalid: Wheel directory entry escapes its destination"

Issue: astral-sh/uv#21694

Classification: bug

## Summary

The reported literal-dot target regression is reproducible. On Linux, uv 0.12.14 fails to install
`annotated-types==0.8.0` with `uv pip install --target=.` and reports that the wheel's normal
`.dist-info` directory escapes its destination. The same command succeeds with uv 0.12.13, and uv
0.12.14 succeeds when the target is a non-dot relative path or an absolute path. Python 3.12.3 is
enough to reproduce the report, so the reported Python 3.14 is not required. A maintainer confirms
that the regression is fixed in uv 0.12.15.

## Classification

This is a bug: a valid command and wheel that succeeded in uv 0.12.13 fails in uv 0.12.14, while
equivalent spellings of the destination continue to work in 0.12.14.

The release boundary is consistent with astral-sh/uv#21569, which merged on 2026-09-10 before uv
0.12.14 and added the destination validation that emits the observed error. The current source
passes the target path through `uv_python::Target` without making it absolute; its wheel validation
then uses `normalize_path_under`, whose tests explicitly reject a root of `.`. This is a
source-supported explanation consistent with the observed literal-dot-only failure, but the
reproduction itself establishes the regression independently of that explanation.

## Reproduction

Outcome: reproducible.

Environment:

- Ubuntu Linux x86_64 (kernel 6.17.0-1022-azure)
- uv 0.12.14 (`x86_64-unknown-linux-gnu`) from `PATH`
- CPython 3.12.3 at `/usr/bin/python3`

From a fresh temporary directory, with the cache also under `/tmp`:

```console
$ uv pip install --python python3 --system --no-compile --no-cache --target=. annotated-types==0.8.0
Using CPython 3.12.3 interpreter at: /usr/bin/python3
Resolved 1 package in 61ms
Prepared 1 package in 2ms
error: Failed to install: annotated_types-0.8.0-py3-none-any.whl (annotated-types==0.8.0)
  cause: The wheel is invalid: Wheel directory entry escapes its destination: annotated_types-0.8.0.dist-info
```

The command exited with status 2. Targeted comparison runs produced:

| uv version | target | Result |
| --- | --- | --- |
| 0.12.14 | `.` | Exit 2 with the reported invalid-wheel error |
| 0.12.14 | `packages` | Exit 0; installed `annotated-types==0.8.0` |
| 0.12.14 | absolute temporary-directory path | Exit 0; installed `annotated-types==0.8.0` |
| 0.12.13 | `.` | Exit 0; installed `annotated-types==0.8.0` |

The neighboring integration coverage in `crates/uv/tests/pip_install/pip_install.rs` includes
`compile_bytecode_for_relative_install_root`, which verifies a non-dot relative target (`target`),
and the astral-sh/uv#21569 symlink-rejection tests such as
`reject_symlinked_wheel_package_directory`. Those tests do not exercise `.` or `./`; the parent
regression tests described below provide the current-directory coverage.

For users who must remain on uv 0.12.14, an absolute target such as `--target="$PWD"` is an observed
workaround.

## Fix

Outcome: fixed and released in uv 0.12.15, as confirmed by a maintainer on astral-sh/uv#21694.

The root cause is that [`Target`] retained current-directory spellings such as `.` unchanged. Wheel
destination validation normalizes that root to an empty path, and [`normalize_path_under`] rejects
an empty root, so ordinary wheel directories were incorrectly classified as escaping the target.
`Target::from` now resolves target paths that normalize to the current directory to [`CWD`]. Other
relative and absolute target paths remain unchanged, and the symlink-destination checks continue to
operate on non-empty roots.

The parent integration tests in `crates/uv/tests/pip_install/pip_install.rs` now require successful
installation into the current directory for both `--target=.` and `target = "."` in
`[tool.uv.pip]`, and assert that the installed package directory exists. Focused validation passed
for both updated regressions, the existing non-dot relative-target bytecode test, the existing
symlinked wheel-destination rejection test, and the pip-sync relative-target install/upgrade/script
round trip. The changed Rust files were formatted with `cargo fmt --all` using the available stable
rustfmt, and focused clippy checks for `uv-python` passed with warnings denied. The fix was developed
in astral-sh/uv-dev#1808.

## Related

- astral-sh/uv#21569 — “Reject symlinked wheel installation destinations” (merged pull request).
  It added `ValidatedWheelDestination` and the exact `Wheel directory entry escapes its
  destination` error path before the 0.12.14 release. It is the introducing change indicated by
  the release boundary and source, not a fix created in response to this issue.
- astral-sh/uv#21692 — a sibling uv 0.12.14 regression in the same destination-validation area.
  Its trigger is a pre-existing `/usr/local/man` symlink and its error is `Cannot install into
  symlinked directory`, so it is distinct from the literal-dot target failure.
