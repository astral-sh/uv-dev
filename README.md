# read_venv_redirect errors unconditionally on empty/invalid .venv when preview features are disabled

Issue: astral-sh/uv#23386

Classification: bug

## Summary

On uv 0.13.9, `uv sync` and project-mode `uv run` produce redirect-specific errors without preview enabled when the project's `.venv` is an empty regular file or points to a missing environment outside uv's centralized cache. The reporter expects uv not to parse or validate redirects when preview is off. The report demonstrates the empty-file case on Windows; both cases reproduced on Linux. Version 0.13.8 also rejects both files, but reports that it expected a directory rather than interpreting them as redirects.

## Draft response

I reproduced both errors without preview enabled. Version 0.13.8 rejected the same files because `.venv` wasn't a directory, so the change is in how uv interprets and reports the files, not whether they prevent sync. Valid redirects work without opting in and emit an experimental warning. astral-sh/uv#23387 proposes a clearer error for an empty file without preview but leaves dangling redirects unchanged. If the `.venv` file is unintended, move it aside and rerun `uv sync`.

## Classification

This is a bug in how project commands handle invalid `.venv` files, not evidence that all redirect support was intended to be opt-in. The maintainer explicitly confirmed the empty-file failure in astral-sh/uv#23340, which was closed over the separate question of replacing the referenced environment. The open astral-sh/uv#23387 overlaps on empty files but does not cover the full project-command report: it changes the error for empty files and does not change missing-target validation. This issue is the place to track the project behavior, not a duplicate of the closed parent or the partial PR.

## Reproduction

**Outcome: reproducible.** The reporter used Windows 11 x86_64, uv 0.13.9 (6b2d39dcc), and Python 3.12.16. The reproduction used the installed `uv` 0.13.9 from `PATH` on Ubuntu 24.04.5 LTS (Linux x86_64) with CPython 3.12.3 at `/usr/bin/python3`. The project has no dependencies or workspace configuration. The runs used an isolated environment, home, and cache, with no preview flags or preview-related environment variables.

In a clean shell with no uv configuration or preview overrides, create the fixture and run:

```sh
work=$(mktemp -d)
mkdir "$work/project"
cd "$work/project"
export UV_CACHE_DIR="$work/cache" UV_NO_CONFIG=1 UV_OFFLINE=1
export UV_PYTHON=/usr/bin/python3 UV_PYTHON_DOWNLOADS=never
printf '[project]\nname = "demo"\nversion = "0.1.0"\n' > pyproject.toml

: > .venv
uv sync
uv run python --version

printf 'missing\n' > .venv
uv sync
uv run python --version
```

- With an empty `.venv`, both commands exit 2: ``error: Virtual environment redirect file at `.venv` is empty``.
- With `.venv` containing `missing` and no such target outside the uv cache, both commands exit 2 with ``cannot be used because no `pyvenv.cfg` file was found`` for the resolved `missing` directory. Neither failure emits the experimental warning.
- After removing the fixture's `.venv` file, `uv sync` exits 0 and creates the environment directory. Moving that directory to `external` and writing `external` to `.venv` makes `uv sync` exit 0 with the experimental redirect warning, still without preview flags.
- The official uv 0.13.8 Linux binary was tested against the same fixture. `uv sync` exits 2 for both file contents with `expected directory but found a file`, naming `.venv` rather than the redirect target. Disabling redirect parsing would not by itself make sync succeed while a regular file occupies that path.

Existing coverage, inspected but not run:

- `crates/uv/tests/sync/centralized_project_envs.rs:677`, `sync_venv_redirect_file`: creates a valid external venv and asserts that plain sync follows the redirect, leaves it in place, and warns. It also asserts success without the warning when the redirect preview feature is enabled.
- `crates/uv/tests/sync/centralized_project_envs.rs:791`, `sync_invalid_venv_redirect_file`: writes `missing` into `.venv`, asserts that locking succeeds, and snapshots the missing-`pyvenv.cfg` sync error **with** preview enabled. It does not cover the no-preview variant.
- `crates/uv-fs/src/lib.rs:1176`, `read_venv_redirect_rejects_empty_path`: checks that the parser returns `InvalidData` for empty and newline-only contents; it does not exercise preview settings or project commands.

## Related

- astral-sh/uv#23340 (closed issue) — The parent report included the same empty-file failure without preview. A maintainer called that a bug but said rebuilding an existing referenced environment was intentional. This issue separates the former from the closed replacement discussion.
- astral-sh/uv#23387 (open pull request) — Proposes skipping empty or non-UTF-8 `.venv` files during Python discovery without preview so discovery can continue to a parent environment. For `uv sync`, it changes the error for an empty project-root `.venv` to say that the path is a file and leaves it in place. It still parses redirects and does not change the missing-target or missing-`pyvenv.cfg` branch.
- astral-sh/uv#21822 (merged pull request) — Introduced PEP 832 redirect support in uv 0.13.9, released November 17, 2026. The implementation and tests show that valid redirects are followed with an experimental warning when the flag is absent. This is the introducing change, not a fix for this report.
- astral-sh/uv#23363 (closed, unmerged pull request) — A PR comment identifies the same early return for dangling targets, but the patch proposed changing how an existing, incompatible external environment is replaced. It was closed without merging and does not fix invalid redirect handling.

## Supporting evidence

- `crates/uv-fs/src/lib.rs:973`: `read_venv_redirect` returns `InvalidData` for an empty file. `resolve_project_env` in `crates/uv-environment-operations/src/lib.rs:444` propagates that error and separately rejects a non-cache target with no `pyvenv.cfg`; both returns precede the preview-warning check at line 474. Pruned centralized-cache targets take a different branch.
- `crates/uv-project-commands/src/sync.rs:214` and `crates/uv-project-commands/src/run.rs:659` both call `ProjectEnvironment::get_or_init`, consistent with the shared diagnostic observed on Linux.

## Search scope

Searched open and closed issues and open, closed, and merged pull requests with `gh`. Literal searches covered `read_venv_redirect`, the empty-file and missing-`pyvenv.cfg` diagnostics, and `venv-redirect-files`. Conceptual searches separated empty or non-directory `.venv` files from dangling targets, then looked at project environment resolution, Python discovery, linked environments, and preview behavior. Fix-oriented searches covered PEP 832 and redirect validation, cross-checked recent PR listings, and followed the parent issue and introducing PR's references, comments, and relevant diffs against the 0.13.9 release. The similar missing-`pyvenv.cfg` error in astral-sh/uv#8092, fixed by astral-sh/uv#9075, concerned `uv init` discovering a parent `.venv` directory, not project commands validating a redirect file. astral-sh/uv#23216 fixed stale interpreter metadata after `pyvenv.cfg` changes, not missing redirect targets.
