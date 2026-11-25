# LenientRequirement missing-dot fixup rewrites requests==2.32.3* to requests==2.3.*

Issue: astral-sh/uv#23406

Classification: bug

## Summary

A malformed wildcard in `[project].dependencies` is repaired to the wrong release prefix. On uv 0.13.10, `uv pip compile pyproject.toml`, `uv lock`, and `uv sync` all select `requests==2.3.0` from `requests==2.32.3*`. Verbose compile output shows the rewrite to `requests==2.3.*`; default compile output does not warn. The reporter also observed the behavior at `main` commit `e3e7c1eacb980339352a5ad69affcd3b6c206c92`, the uv 0.13.10 version bump.

## Classification

This is a bug: although the input is malformed, the repair changes the release prefix instead of preserving it or rejecting the input. The same compile result was observed on uv 0.13.9, so the behavior was not introduced by 0.13.10. The earlier triage found no duplicate or prior fix for this truncation. Warning visibility is separate from the incorrect rewrite.

## Reproduction

**Outcome: reproducible.** Tested the published uv 0.13.10 binary with CPython 3.14.10 on Linux 7.0.0-1017-azure x86_64 (the reporter used Linux 6.1.0-56-cloud-amd64 x86_64). The commands used public PyPI, a temporary home and cache, and no external uv configuration. The uv release and Python interpreter were installed in temporary directories; no binary was built from the checkout.

In a fresh temporary project directory, write this `pyproject.toml`:

```toml
[project]
name = "example"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = ["requests==2.32.3*"]
```

Run with uv 0.13.10 and Python 3.14.10:

```sh
uv --no-config pip compile pyproject.toml --python 3.14.10 --no-build
uv --no-config pip compile pyproject.toml -v --python 3.14.10 --no-build
uv --no-config lock --python 3.14.10 --no-build
# Run sync in a separate fresh directory containing only the same pyproject.toml.
uv --no-config sync --python 3.14.10 --no-build
uv --no-config pip show --python .venv/bin/python requests
```

- Default compile output pins `requests==2.3.0` without a repair warning. With `-v`, uv logs “Fixing invalid requirement by inserting missing dot,” naming `requests==2.32.3*` as the input and `requests==2.3.*` as the result, and selects the same version.
- `uv lock` records `==2.3.*` in `requires-dist` and locks `requests` to `2.3.0`. `uv sync` from scratch installs `2.3.0`, confirmed by `uv pip show`.
- As controls, `requests==2.2.0*` is repaired to `requests==2.2.0.*` and resolves `2.2.0`; the valid `requests==2.32.3.*` resolves `2.32.3`.
- The installed uv 0.13.9 on `PATH`, tested with CPython 3.12.3 on the same host, also pins `2.3.0` and shows the same incorrect rewrite with `-v`.
- The same fixup affects other metadata consumers. In the parent regression, `data-requires-python="==3.14.1*"` in local HTML find-links metadata became `==4.1.*` and excluded a wheel from a Python 3.14.1 resolution. In a local environment with `Requires-Dist: child==2.32.3*` in an installed package's `METADATA` and `child==2.3.0` installed, unpatched `uv pip install --no-index --no-deps --strict parent` reported no incompatibility. The valid `Requires-Dist: child==2.32.3.*` control warned about the incompatible installation.
- Existing unit tests `requirement_missing_dot` (`crates/uv-pypi-types/src/lenient_requirement.rs:215`) and `specifier_missing_dot` (`crates/uv-pypi-types/src/lenient_requirement.rs:280`) assert that short exclusions such as `!=3.0*` become `!=3.0.*`. The parent integration tests added coverage for the multi-digit cases.

## Fix

**Outcome: fixed.** The shared missing-dot rule in `crates/uv-pypi-types/src/lenient_requirement.rs:34` now captures the entire matched numeric substring rather than the last repetition. The replacement only inserts the missing dot: `2.32.3*` becomes `2.32.3.*`, and `3.14.1*` becomes `3.14.1.*`. The rule matches the same input shape, and the fixup order and logging level are unchanged.

All three parent tests passed with the original snapshots. After changing their expectations, they failed on the wrong pin, the truncated lockfile requirement, and the incompatible Python check before the fix. The updated installed-metadata test also failed on the truncated requirement in its strict diagnostic. After the fix, the following focused debug-profile integration checks passed:

- `compile_pyproject_toml_dependency_missing_wildcard_dot` pins `requests==2.32.3`, and `compile_find_links_requires_python_missing_wildcard_dot` accepts the compatible wheel for Python 3.14.1.
- `lock_dependency_missing_wildcard_dot` records `==2.32.3.*`, locks `2.32.3`, and installs `2.32.3` when `uv sync --frozen` consumes the lockfile.
- The existing `no_deps_installed` test uses the malformed installed metadata and confirms strict mode still flags both missing and incompatible dependencies. An installed `2.3.0` no longer satisfies the repaired prefix.
- `invalid_metadata_requires_python` and `lock_no_build_invalid_dependency_virtual_project` still reject unrelated malformed inputs.

The two existing `missing_dot` unit tests also passed, as did `cargo fmt --all --check`.

## Workaround

On affected uv versions, use `requests==2.32.3.*` for a prefix match or `requests==2.32.3` for an exact pin.

## Related

- astral-sh/uv#408 (closed) — “Accept more invalid version specifiers” requested recovery for PyPI metadata such as `!=3.0*`, `!=3.1*`, and `!=3.2*`, as well as trailing commas. It concerned parse failures on short inputs, not a successful rewrite that drops digits.
- astral-sh/uv#416 (merged) — “Add missing-dot fix to lenient requirements” introduced the same repeated capture and replacement used by the parser at the reported revision. Its test checks the short exclusions from astral-sh/uv#408 and cannot expose truncation in a multi-digit release prefix.
- astral-sh/uv#775 (merged) — “Filter out files with invalid requires python specifiers” switched shared fixup diagnostics from a user-facing warning to `tracing::warn!`. That explains the separate visibility symptom; the PR did not change the missing-dot regex or address the wrong release prefix.
- astral-sh/uv-dev#3173 — Parent regression coverage for project dependency compilation, lock and frozen sync, and Python compatibility from HTML link metadata.

## Implementation

- Static project metadata passes `project.dependencies` through `LenientRequirement` (`crates/uv-pypi-types/src/metadata/metadata_resolver.rs:234`). The original rule matched `2.32.3*` as two repetitions of `2.3`, then kept only the last capture with `${1}.*`. For `2.2.0*`, it matched only `2.0*`, leaving the leading `2.` intact. Both `LenientRequirement` and `LenientVersionSpecifiers` use the same fixup table.
- The repair emits `tracing::warn!`; the CLI defaults to tracing off unless configured otherwise and enables uv tracing with `-v` (`crates/uv/src/lib.rs:194`, `crates/uv/src/logging.rs:39`).

## Search scope

Earlier triage searched open and closed issues and open, closed, and merged PRs. Literal queries used the input and output versions, `LenientRequirement`, `LenientVersionSpecifiers`, and “Fixing invalid requirement” / “inserting missing dot.” Conceptual queries covered wrong-version selection, wildcard prefix matching, malformed specifiers and metadata, the project commands, and verbose-only warnings. Fix-oriented checks included PR listings, linked discussions, merged diffs, and current and historical parser paths.

Ruled out astral-sh/uv#21011 and astral-sh/uv#21012: that issue and fix allow whitespace before a version with an existing `.*` on an ordered comparator such as `>=`, using a different rule. Issues and PRs astral-sh/uv#1477, astral-sh/uv#1529, astral-sh/uv#2546, and astral-sh/uv#2550 concern ordered-comparator metadata and interactions with quote stripping, not losing digits while adding a dot. Issue astral-sh/uv#8326 concerns rejection of bare `==*` with no release prefix.

Pull request: https://github.com/astral-sh/uv-dev/pull/3174
