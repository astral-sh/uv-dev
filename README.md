# read_venv_redirect errors unconditionally on empty/invalid .venv when preview features are disabled

Issue: astral-sh/uv#23386

Classification: bug

## Summary

On uv 0.13.9, `uv sync` and project-mode `uv run` can fail without preview flags if the project's `.venv` is an empty regular file or points to a missing or invalid environment outside uv's centralized cache. The report gives an empty-file `uv sync` reproduction on Windows and describes the dangling-target case. astral-sh/uv#23340 confirms the empty-file bug, astral-sh/uv#21822 introduced the redirect behavior, and astral-sh/uv#23387 proposes a partial change that leaves dangling targets unchanged.

## Draft response

Confirmed: `uv sync` and project-mode `uv run` can report a redirect error before checking the preview setting. Valid redirects already work without the flag; the flag suppresses their experimental warning. astral-sh/uv#23387 proposes reporting an empty project-root `.venv` as a file blocking the environment directory, but it leaves dangling redirects unchanged. Let's track the project-command behavior here. If the `.venv` file is unintended, move it aside and rerun `uv sync`.

## Classification

This is a bug in how project commands handle invalid `.venv` files, not evidence that all redirect support was intended to be opt-in. The maintainer explicitly confirmed the empty-file failure in astral-sh/uv#23340, which was closed over the separate question of replacing the referenced environment. The open astral-sh/uv#23387 overlaps on empty files but does not cover the full project-command report: it changes the error for empty files and does not change missing-target validation. This issue is the place to track the project behavior, not a duplicate of the closed parent or the partial PR.

## Related

- astral-sh/uv#23340 (closed issue) — The parent report included the same empty-file failure without preview. A maintainer called that a bug but said rebuilding an existing referenced environment was intentional. This issue separates the former from the closed replacement discussion.
- astral-sh/uv#23387 (open pull request) — Proposes skipping empty or non-UTF-8 `.venv` files during Python discovery without preview so discovery can continue to a parent environment. For `uv sync`, it changes the error for an empty project-root `.venv` to say that the path is a file and leaves it in place. It still parses redirects and does not change the missing-target or missing-`pyvenv.cfg` branch.
- astral-sh/uv#21822 (merged pull request) — Introduced PEP 832 redirect support in uv 0.13.9, released November 17, 2026. The implementation and tests show that valid redirects are followed with an experimental warning when the flag is absent. This is the introducing change, not a fix for this report.
- astral-sh/uv#23363 (closed, unmerged pull request) — A PR comment identifies the same early return for dangling targets, but the patch proposed changing how an existing, incompatible external environment is replaced. It was closed without merging and does not fix invalid redirect handling.

## Supporting evidence

- `crates/uv-fs/src/lib.rs:973`: `read_venv_redirect` returns `InvalidData` for an empty file. `resolve_project_env` in `crates/uv-environment-operations/src/lib.rs:444` propagates that error and separately rejects a non-cache target with no `pyvenv.cfg`; both returns precede the preview-warning check at line 474. Pruned centralized-cache targets take a different branch.
- `crates/uv-project-commands/src/sync.rs:214` and `crates/uv-project-commands/src/run.rs:659` both call `ProjectEnvironment::get_or_init`. The shared resolver is not Windows-specific. These findings are based on source and test inspection, not a new runtime reproduction.
- `crates/uv/tests/sync/centralized_project_envs.rs:677` covers a valid redirect with no preview flags and expects the warning. The missing-target case at line 791 asserts the `pyvenv.cfg` error with preview enabled; it does not test that case without preview. A regular file at the project environment path still blocks creating a directory there, even if it is not reported as an invalid redirect.

## Search scope

Searched open and closed issues and open, closed, and merged pull requests with `gh`. Literal searches covered `read_venv_redirect`, the empty-file and missing-`pyvenv.cfg` diagnostics, and `venv-redirect-files`. Conceptual searches separated empty or non-directory `.venv` files from dangling targets, then looked at project environment resolution, Python discovery, linked environments, and preview behavior. Fix-oriented searches covered PEP 832 and redirect validation, cross-checked recent PR listings, and followed the parent issue and introducing PR's references, comments, and relevant diffs against the 0.13.9 release. The similar missing-`pyvenv.cfg` error in astral-sh/uv#8092, fixed by astral-sh/uv#9075, concerned `uv init` discovering a parent `.venv` directory, not project commands validating a redirect file. astral-sh/uv#23216 fixed stale interpreter metadata after `pyvenv.cfg` changes, not missing redirect targets.
