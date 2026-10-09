# `uv run --with` packages invisible to project entrypoints when the venv path forces the /bin/sh shebang wrapper

Issue: astral-sh/uv#22323

Classification: bug

## Summary

The checkout now contains a focused fix for absolute shell-wrapper entrypoint copying and updated parent regressions that require successful imports from the overlay. See the fix validation below.

A project entrypoint launched with `uv run --with` can use the project's base interpreter instead of the ephemeral overlay interpreter. Consequently, packages requested with `--with` are unavailable. The supplied example installs pytest in the project and requests six only for the invocation: `uv run --with six pytest -q test_x.py` raises `ModuleNotFoundError: No module named 'six'`, while invoking `python -m pytest` succeeds.

The trigger is a POSIX interpreter path that makes uv generate an absolute `/bin/sh` wrapper. This occurs when the complete direct shebang would exceed 127 bytes, including `#!` and its newline, or when the interpreter path contains a space. Independent reproduction confirms both long-path and space-containing-path failures on Linux x86_64 with the installed uv 0.12.13 and CPython 3.12.3. The short-path control and the long-path `python -m pytest` workaround both pass. Runtime trace output confirms that uv skips copying the absolute shell-wrapper entrypoint, and `sys.executable` confirms that the failed invocation uses the project interpreter. The reporter tested macOS aarch64 with uv 0.11.32 and 0.12.17; a subsequent issue comment reports Linux results with uv 0.12.17 and main at e8c3128f0. Those other versions and macOS were not independently executed here.

Closest history: astral-sh/uv#13327 and its fix astral-sh/uv#14790, followed by the distinct shebang-matching fix astral-sh/uv#14970. Open reports astral-sh/uv#12691 and astral-sh/uv#9724 describe pre-fix behavior; no existing report or PR specifically covering the remaining absolute-wrapper failure was found.

## Draft response

Reproduced on Linux x86_64 with uv 0.12.13 and Python 3.12.3. With a long project path, `uv run --with six==1.17.0 pytest -q -s test_x.py` fails with `ModuleNotFoundError: No module named 'six'`; the same project at a short path passes. A short path containing a space also fails. Trace output confirms that uv skips copying the absolute /bin/sh launcher, and the failing test reports the project interpreter rather than the overlay interpreter.

The `uv run --with six==1.17.0 python -m pytest -q -s test_x.py` workaround passes at the long path. This is an uncovered case in the entrypoint rewriting introduced by astral-sh/uv#14790. The checkout fix recognizes and rewrites these absolute wrappers. Both parent regressions now require successful imports, including a space-containing environment path with an apostrophe.

## Reproduction

**Outcome: reproducible.** Tested using the installed `uv` from `PATH`, not a checkout build:

- uv: `0.12.13 (x86_64-unknown-linux-gnu)`, executable `/opt/hostedtoolcache/uv/0.12.13/x86_64/uv`.
- Platform: Linux `6.17.0-1022-azure`, x86_64; CPython 3.12.3 at `/usr/bin/python3`.
- Project dependency: `pytest==8.3.5`; invocation-only dependency: `six==1.17.0`.
- Standalone project with no workspace, dependency groups, custom sources, or build backend. Commands use normal project synchronization, without `--frozen`.
- All fixtures, generated environments, caches, configuration directories, and command logs are under `/tmp/uv-22323-woppv2ux`. Subprocesses used a clean allowlisted environment, public PyPI, disabled user configuration, disabled Python downloads, and disabled automatic pytest plugin loading.

The following reconstructs the tested long-path fixture. The extra prints and `-s` identify the interpreter selected by the launcher; the failing operation is simply `import six`.

```bash
repro_root=$(mktemp -d /tmp/uv-22323-XXXXXXXX)
export UV_CACHE_DIR="$repro_root/cache"
export UV_PYTHON_INSTALL_DIR="$repro_root/python"
export UV_PYTHON=/usr/bin/python3
export UV_PYTHON_DOWNLOADS=never UV_NO_CONFIG=1 UV_NO_PROGRESS=1
export UV_DEFAULT_INDEX=https://pypi.org/simple
export PYTEST_DISABLE_PLUGIN_AUTOLOAD=1
export XDG_CONFIG_HOME="$repro_root/config"
export XDG_DATA_HOME="$repro_root/data"
export XDG_CACHE_HOME="$repro_root/xdg-cache"
export TMPDIR="$repro_root/tmp"
mkdir -p "$TMPDIR"
long_component=$(python3 -c 'print("a" * 160)')
mkdir -p "$repro_root/$long_component/project"
cd "$repro_root/$long_component/project"
cat > pyproject.toml <<'TOML'
[project]
name = "repro"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["pytest==8.3.5"]
TOML
cat > test_x.py <<'PYTHON'
def test_x():
    import sys
    print(f"interpreter: {sys.executable}")
    import six
    print(f"six: {six.__version__}")
PYTHON
uv sync
head -n 3 .venv/bin/pytest
uv run --with six==1.17.0 pytest -q -s test_x.py
uv run --with six==1.17.0 python -m pytest -q -s test_x.py
```

`uv sync` succeeds. The generated pytest launcher starts with the following, where `<project>` is the absolute long project directory:

```text
#!/bin/sh
'''exec' '<project>/.venv/bin/python' "$0" "$@"
' '''
```

The equivalent direct shebang would be 211 bytes including `#!` and the newline. `uv run --with six==1.17.0 pytest -q -s test_x.py` exits 1 with `1 failed` and `ModuleNotFoundError: No module named 'six'`. Its printed interpreter is `<project>/.venv/bin/python`. The module invocation exits 0 with `1 passed`, prints `six: 1.17.0`, and selects `$UV_CACHE_DIR/builds-v0/<temporary-directory>/bin/python`.

Additional explicit invocations used identical `pyproject.toml` and `test_x.py` files in separate directories:

| Scenario | Launcher | Result of `uv run --with six==1.17.0 pytest -q -s test_x.py` |
| --- | --- | --- |
| Long path: 160-character component followed by `project` | Absolute `/bin/sh` wrapper; equivalent direct shebang 211 bytes | Exit 1; project interpreter; `six` missing |
| Short path: `/tmp/uv-22323-woppv2ux/short` | Direct Python shebang, 48 bytes | Exit 0; overlay interpreter; `six==1.17.0`; `1 passed` |
| Space-containing path: `/tmp/uv-22323-woppv2ux/with space` | Absolute `/bin/sh` wrapper despite equivalent direct shebang being only 53 bytes | Exit 1; project interpreter; `six` missing |

An offline diagnostic rerun at the long path, `uv -vv run --offline --with six==1.17.0 pytest -q test_x.py`, also exits 1 and emits:

```text
TRACE Skipping copy of entrypoint `.venv/bin/pytest`: does not start with expected shebang
```

These results directly confirm the reported failure, the launcher-selection mechanism, and the module-invocation workaround on the tested platform. They do not establish a regression between releases: no last known-good version is supplied, and the reported uv 0.11.32/0.12.17 versions were not executed here.

During initial reproduction, existing test coverage was inspected, including fixture setup and assertions:

- `crates/uv/tests/project/run.rs`, `run_with_overlay_interpreter`: creates a project console entrypoint, snapshots the overlay interpreter and rewritten launcher, checks imports of the project and overlay dependency through `python`, then repeats entrypoint checks after switching to a relocatable environment. It does not place the project at a long or space-containing path and does not test the absolute shell-wrapper interaction.
- `crates/uv-install-wheel/src/wheel.rs`, `test_shebang`: directly asserts launcher formatting for ordinary, space-containing, long, and relocatable paths. It does not exercise `uv run --with` or entrypoint copying.
- Initial searches in `crates/uv/tests/` and `crates/uv-client/tests/it/` found no integration scenario covering the absolute shell-wrapper interaction with an overlay-only dependency. Parent regression PR astral-sh/uv-dev#2582 subsequently added the two scenarios updated by this fix.

The initial reproduction changed no checkout files and used isolated pytest experiments rather than checkout builds or Rust tests. Its logs, lockfiles, settings, and exit results were stored in the temporary reproduction directory; that directory is not present in the subsequent fix environment. The checkout changes and debug-profile validation are described below.

## Classification

Observed execution and baseline source confirm a correctness bug: in affected versions, the installer emits absolute /bin/sh wrappers for long or space-containing POSIX interpreter paths, but copy_entrypoint skips those launchers, allowing the base interpreter to bypass the --with overlay. The omission was already present in astral-sh/uv#14790, so this is an uncovered case in the historical fix, not a demonstrated regression. The older open reports predate that fix and do not establish an existing discussion of this current wrapper-specific failure.

The fact that astral-sh/uv#12691 and astral-sh/uv#9724 remain open does not demonstrate that they track the present defect. Their latest comments are dated May 27, 2025 and December 11, 2024 respectively, before the July 22, 2025 entrypoint-copying fix. The original patch for that fix already lacks the absolute shell-wrapper form. There is no evidence here that this form worked in a released version and subsequently stopped working.

## Fix

**Outcome: fixed in the checkout.** The parent regression PR is astral-sh/uv-dev#2582; its starting commit is `3c6b6ae2258bb3486d4964047308fc7f5aa2bbd7`.

`crates/uv-project-commands/src/run.rs` now recognizes the complete absolute `/bin/sh` wrapper generated by the wheel installer, in addition to direct and relocatable shebangs. Matching is tied to the expected interpreter path and uses the installer's `escape_posix_for_single_quotes` helper. The existing `python3` to `python` fallback applies to both absolute forms. Once matched, the entire wrapper is replaced with the overlay interpreter shebang, so the entrypoint can import invocation-only dependencies. Unrecognized launchers, script permissions, and overwrite handling are unchanged.

Both parent regressions in `crates/uv/tests/project/run.rs` were first verified to pass while expecting the undesirable import failure. Their snapshots were then changed to require exit 0 and `1 passed`. Before production changes, both updated tests failed specifically with `ModuleNotFoundError: No module named 'six'`; their installation and generated-wrapper snapshots passed. The existing ordinary/relocatable test remained green.

Inspection included `format_shebang`, both the console-entrypoint and wheel-script installation paths, interpreter-name fallback, and neighboring coverage in project run, pip install, and virtual-environment tests. An independent experiment with a space and apostrophe in the environment path reproduced the same missing import and exposed the escaped wrapper form. The existing space-path parent fixture was extended to cover it, without adding a duplicate scenario. Other inspected tests, including `install_relocatable` and `strip_shebang_arguments`, did not encode the incorrect overlay assumption and were left unchanged. Other commands reusing these paths were not added to the coverage matrix.

Successful focused validation on Linux x86_64 with CPython 3.12.9:

- `INSTA_UPDATE=no cargo test -p uv --test project run::run_with_overlay_ --locked`: all three tests passed in the unoptimized debug test profile. This includes both installer-to-overlay execution round trips, the module control, and ordinary/relocatable entrypoint coverage.
- `RUSTUP_TOOLCHAIN=stable cargo fmt --all`: completed with the installed rustfmt 1.10.0. The pinned toolchain lacked rustfmt and its component store was read-only, so formatting used the installed stable toolchain.
- `git diff --check`: passed. Only the affected production file and the original parent integration-test file are modified.

No release builds, full test suites, dependency changes, commits, pushes, or GitHub modifications were performed.

## Related

- astral-sh/uv-dev#2582 (parent regression pull request) — Adds the long-project-path and space-containing-environment regressions updated by this fix.

- astral-sh/uv#13327 (issue, closed) — Ephemeral environments lacking scripts entry points. Historical report of project entrypoints using the base interpreter and missing --with packages, with python -m pytest as a workaround. Closed by astral-sh/uv#14790 in uv 0.8.1; the new report identifies an absolute shell-wrapper case omitted by that fix.
- astral-sh/uv#14790 (pull request, merged) — Copy entry points and Jupyter data directories into ephemeral environments. Introduced entrypoint copying and interpreter rewriting in uv 0.8.1 to fix astral-sh/uv#13327. Its patch recognizes direct Python shebangs and the relocatable shell wrapper, but already omits the absolute shell wrapper. This establishes an incomplete historical fix rather than evidence that wrapper support later regressed.
- astral-sh/uv#14970 (pull request, merged) — Copy entrypoints that have a shebang that differs in `python` vs `python3`. A follow-up matcher fix released in uv 0.8.4: entrypoints were skipped when the discovered executable ended in python3 but the direct shebang used python. It modifies the same copying logic but does not handle absolute /bin/sh wrappers.
- astral-sh/uv#12691 (issue, open) — `uv run --with` uses different dependencies when prefixed with `python`. Closely matches the pytest versus python -m pytest symptom, but reports uv 0.6.12 and was last discussed in May 2025, before entrypoint rewriting shipped in uv 0.8.1. It contains no evidence of the current long-path or space-triggered wrapper omission, so it is historical context rather than an established duplicate target.
- astral-sh/uv#9724 (issue, open) — `--with` not working with local packages. Reports a project console script missing a local --with dependency because its shebang selects the base environment. Its uv 0.5.7 reproduction and December 2024 discussion predate astral-sh/uv#14790; it does not establish that the later absolute-wrapper omission is already tracked.

## Supporting evidence

The following baseline source findings refer to commit `01b62808962d7abfe2d10f43d652f357d8038202`, before the fix described above.

- `crates/uv-install-wheel/src/wheel.rs:110`: `format_shebang` selects the shell wrapper for POSIX paths when `2 + executable.len() + 1 > 127`, when the path contains a space, or when relocation is requested. Non-relocatable paths produce an absolute quoted executable in the wrapper; relocatable paths use the `dirname`/`realpath` prefix.
- `crates/uv-project-commands/src/run.rs:1035`: the ephemeral environment receives a `.pth` overlay that exposes both the requirements and base environments. At line 1064, scripts from both environments are copied with their interpreter target rewritten.
- `crates/uv-project-commands/src/run.rs:2083`: `copy_entrypoint` accepts the specific relocatable shell wrapper, a direct Python shebang, and the direct `python` spelling when the discovered executable ends in `python3`. It has no branch for the absolute shell wrapper. Nonmatches return without creating the overlay entrypoint and emit the reported trace message.
- `crates/uv-project-commands/src/run.rs:1233`: command lookup searches the ephemeral scripts directory, then requirements scripts, then base scripts. If the project entrypoint is skipped and absent from the requirements environment, the original base launcher remains discoverable and explicitly starts the base interpreter.
- `docs/concepts/projects/run.md:25`: additional or overridden dependencies are intended to apply to the invocation. The requested package becoming unavailable solely because the project path selects a different supported launcher format violates that behavior.
- The patch and review discussion for astral-sh/uv#14790 establish that direct and relocatable shebangs were supported from the start; the absolute shell wrapper was omitted. The uv 0.8.1 release notes announce entrypoint copying to respect environment layers. The uv 0.8.4 release notes identify astral-sh/uv#14970 as the separate direct `python` versus `python3` matching fix.

## Integration coverage

The updated parent tests in `crates/uv/tests/project/run.rs` cover installation of absolute shell wrappers followed by execution with an overlay-only dependency. `run_with_overlay_long_project_path` retains its generated-launcher snapshot and module-invocation control. `run_with_overlay_space_in_environment_path` retains the space trigger and additionally checks the installer's escaped apostrophe in `custom 'venv`.

`run_with_overlay_interpreter` checks the ordinary entrypoint, rewritten script contents, overlay interpreter, and relocatable environment. All three tests passed together after the fix. The module is gated on `test-python` and `test-pypi`; both wrapper-specific parent tests retain their Unix gates. No new tests or feature gates were needed.

## Search scope and exclusions

Searched astral-sh/uv open and closed issues using exact identifiers and error fragments (copy_entrypoint, format_shebang, ModuleNotFoundError, Skipping copy of entrypoint, does not start with expected shebang), command comparisons (uv run --with, pytest, python -m), and conceptual terms (ephemeral environments, entry points, console scripts, shebangs, long paths, spaces, shell wrappers), including environment/project label searches. Searched open, closed, and merged PRs for shebang, entrypoint, ephemeral, and the issue number. PR keyword searches returned no hits even for known historical work, so supplemented them by scanning all 602 open PRs and the latest 1,000 closed/merged PR titles, and following issue links to historical PRs, comments, review discussions, patches, and uv 0.8.1/0.8.4 release notes. Inspected astral-sh/uv#12691 and astral-sh/uv#9724 but ruled them out as established current duplicate targets because their discussions predate entrypoint copying. Also ruled out astral-sh/uv#15113 (system site-package paths), astral-sh/uv#9348 (upstream launcher quoting), astral-sh/uv#14877 (native binary entrypoints), and astral-sh/uv#21280 (source encoding declarations displaced by shell wrappers).

Other inspected results reinforced these distinctions: astral-sh/uv#14729 and astral-sh/uv#14749 concern the earlier overlay/entrypoint breakage resolved by astral-sh/uv#14790; astral-sh/uv#14919 led to the direct interpreter-name fix in astral-sh/uv#14970. astral-sh/uv#15219 concerns missing Jupyter data templates, astral-sh/uv#11048 concerns interpreter-path canonicalization, and astral-sh/uv#21077 concerns an incorrect base interpreter reported by copied CPython environments. None establishes the same absolute-wrapper omission.

PR keyword search did not return known matching historical PRs, which limits confidence in those search results alone. Direct PR listings, linked discussions, patches, and release notes supplied the additional evidence above. No exact current duplicate or wrapper-specific fix was identified within that scope.

Pull request: https://github.com/astral-sh/uv-dev/pull/2584
