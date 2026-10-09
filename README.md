# `uv run --with` packages invisible to project entrypoints when the venv path forces the /bin/sh shebang wrapper

Issue: astral-sh/uv#22323

Classification: bug

## Summary

A project entrypoint launched with `uv run --with` can use the project's base interpreter instead of the ephemeral overlay interpreter. Consequently, packages requested with `--with` are unavailable. The supplied example installs pytest in the project and requests six only for the invocation: `uv run --with six pytest -q test_x.py` raises `ModuleNotFoundError: No module named 'six'`, while invoking `python -m pytest` succeeds.

The trigger is a POSIX interpreter path that makes uv generate an absolute `/bin/sh` wrapper. This occurs when the complete direct shebang would exceed 127 bytes, including `#!` and its newline, or when the interpreter path contains a space. The reporter tested macOS aarch64 with uv 0.11.32 and 0.12.17. A subsequent issue comment reports reproducing both long-path and space-containing-path cases on Linux with uv 0.12.17 and main at e8c3128f0. Those execution results are contributor reports; this handoff independently verifies the source mismatch.

Closest history: astral-sh/uv#13327 and its fix astral-sh/uv#14790, followed by the distinct shebang-matching fix astral-sh/uv#14970. Open reports astral-sh/uv#12691 and astral-sh/uv#9724 describe pre-fix behavior; no existing report or PR specifically covering the remaining absolute-wrapper failure was found.

## Draft response

Your diagnosis matches the current code: uv generates absolute /bin/sh wrappers for long paths and paths containing spaces, but entrypoint copying does not recognize that form. The launcher therefore keeps using the project interpreter and misses the --with packages. This is an uncovered case in the entrypoint rewriting introduced by astral-sh/uv#14790.

Your `uv run --with six python -m pytest -q test_x.py` workaround avoids the skipped launcher. The fix should recognize and rewrite these absolute wrappers, with focused integration coverage for long paths and paths containing spaces.

## Classification

Current source confirms a correctness bug: the installer emits absolute /bin/sh wrappers for long or space-containing POSIX interpreter paths, but copy_entrypoint skips those launchers, allowing the base interpreter to bypass the --with overlay. The omission was already present in astral-sh/uv#14790, so this is an uncovered case in the historical fix, not a demonstrated regression. The older open reports predate that fix and do not establish an existing discussion of this current wrapper-specific failure.

The fact that astral-sh/uv#12691 and astral-sh/uv#9724 remain open does not demonstrate that they track the present defect. Their latest comments are dated May 27, 2025 and December 11, 2024 respectively, before the July 22, 2025 entrypoint-copying fix. The original patch for that fix already lacks the absolute shell-wrapper form. There is no evidence here that this form worked in a released version and subsequently stopped working.

## Related

- astral-sh/uv#13327 (issue, closed) — Ephemeral environments lacking scripts entry points. Historical report of project entrypoints using the base interpreter and missing --with packages, with python -m pytest as a workaround. Closed by astral-sh/uv#14790 in uv 0.8.1; the new report identifies an absolute shell-wrapper case omitted by that fix.
- astral-sh/uv#14790 (pull request, merged) — Copy entry points and Jupyter data directories into ephemeral environments. Introduced entrypoint copying and interpreter rewriting in uv 0.8.1 to fix astral-sh/uv#13327. Its patch recognizes direct Python shebangs and the relocatable shell wrapper, but already omits the absolute shell wrapper. This establishes an incomplete historical fix rather than evidence that wrapper support later regressed.
- astral-sh/uv#14970 (pull request, merged) — Copy entrypoints that have a shebang that differs in `python` vs `python3`. A follow-up matcher fix released in uv 0.8.4: entrypoints were skipped when the discovered executable ended in python3 but the direct shebang used python. It modifies the same copying logic but does not handle absolute /bin/sh wrappers.
- astral-sh/uv#12691 (issue, open) — `uv run --with` uses different dependencies when prefixed with `python`. Closely matches the pytest versus python -m pytest symptom, but reports uv 0.6.12 and was last discussed in May 2025, before entrypoint rewriting shipped in uv 0.8.1. It contains no evidence of the current long-path or space-triggered wrapper omission, so it is historical context rather than an established duplicate target.
- astral-sh/uv#9724 (issue, open) — `--with` not working with local packages. Reports a project console script missing a local --with dependency because its shebang selects the base environment. Its uv 0.5.7 reproduction and December 2024 discussion predate astral-sh/uv#14790; it does not establish that the later absolute-wrapper omission is already tracked.

## Supporting evidence

Source inspection used checkout commit `01b62808962d7abfe2d10f43d652f357d8038202`.

- `crates/uv-install-wheel/src/wheel.rs:110`: `format_shebang` selects the shell wrapper for POSIX paths when `2 + executable.len() + 1 > 127`, when the path contains a space, or when relocation is requested. Non-relocatable paths produce an absolute quoted executable in the wrapper; relocatable paths use the `dirname`/`realpath` prefix.
- `crates/uv-project-commands/src/run.rs:1035`: the ephemeral environment receives a `.pth` overlay that exposes both the requirements and base environments. At line 1064, scripts from both environments are copied with their interpreter target rewritten.
- `crates/uv-project-commands/src/run.rs:2083`: `copy_entrypoint` accepts the specific relocatable shell wrapper, a direct Python shebang, and the direct `python` spelling when the discovered executable ends in `python3`. It has no branch for the absolute shell wrapper. Nonmatches return without creating the overlay entrypoint and emit the reported trace message.
- `crates/uv-project-commands/src/run.rs:1233`: command lookup searches the ephemeral scripts directory, then requirements scripts, then base scripts. If the project entrypoint is skipped and absent from the requirements environment, the original base launcher remains discoverable and explicitly starts the base interpreter.
- `docs/concepts/projects/run.md:25`: additional or overridden dependencies are intended to apply to the invocation. The requested package becoming unavailable solely because the project path selects a different supported launcher format violates that behavior.
- The patch and review discussion for astral-sh/uv#14790 establish that direct and relocatable shebangs were supported from the start; the absolute shell wrapper was omitted. The uv 0.8.1 release notes announce entrypoint copying to respect environment layers. The uv 0.8.4 release notes identify astral-sh/uv#14970 as the separate direct `python` versus `python3` matching fix.

## Suggested next step and existing coverage

Extend entrypoint recognition and rewriting to handle the absolute shell-wrapper form generated by uv. Keep matching tied to the expected interpreter and its established `python`/`python3` handling. The reporter's module invocation is a usable workaround for pytest.

`run_with_overlay_interpreter` in `crates/uv/tests/project/run.rs:1708` already covers ordinary and relocatable project entrypoints. `test_shebang` in `crates/uv-install-wheel/src/wheel.rs:1378` covers generation of wrappers for long paths and spaces. The missing coverage is their interaction: executing a project entrypoint through the overlay when its original launcher uses an absolute shell wrapper.

Add separate, explicitly named POSIX integration scenarios for long project paths and project paths containing spaces. Verify access to an overlay-only dependency through the entrypoint using the existing `uv_snapshot!` style. The project run module is already gated on `test-python` and `test-pypi`; wrapper-specific cases also need a Unix platform gate. This handoff proposes coverage only; no source or test files were changed and no builds or test executions were performed.

## Search scope and exclusions

Searched astral-sh/uv open and closed issues using exact identifiers and error fragments (copy_entrypoint, format_shebang, ModuleNotFoundError, Skipping copy of entrypoint, does not start with expected shebang), command comparisons (uv run --with, pytest, python -m), and conceptual terms (ephemeral environments, entry points, console scripts, shebangs, long paths, spaces, shell wrappers), including environment/project label searches. Searched open, closed, and merged PRs for shebang, entrypoint, ephemeral, and the issue number. PR keyword searches returned no hits even for known historical work, so supplemented them by scanning all 602 open PRs and the latest 1,000 closed/merged PR titles, and following issue links to historical PRs, comments, review discussions, patches, and uv 0.8.1/0.8.4 release notes. Inspected astral-sh/uv#12691 and astral-sh/uv#9724 but ruled them out as established current duplicate targets because their discussions predate entrypoint copying. Also ruled out astral-sh/uv#15113 (system site-package paths), astral-sh/uv#9348 (upstream launcher quoting), astral-sh/uv#14877 (native binary entrypoints), and astral-sh/uv#21280 (source encoding declarations displaced by shell wrappers).

Other inspected results reinforced these distinctions: astral-sh/uv#14729 and astral-sh/uv#14749 concern the earlier overlay/entrypoint breakage resolved by astral-sh/uv#14790; astral-sh/uv#14919 led to the direct interpreter-name fix in astral-sh/uv#14970. astral-sh/uv#15219 concerns missing Jupyter data templates, astral-sh/uv#11048 concerns interpreter-path canonicalization, and astral-sh/uv#21077 concerns an incorrect base interpreter reported by copied CPython environments. None establishes the same absolute-wrapper omission.

PR keyword search did not return known matching historical PRs, which limits confidence in those search results alone. Direct PR listings, linked discussions, patches, and release notes supplied the additional evidence above. No exact current duplicate or wrapper-specific fix was identified within that scope.
