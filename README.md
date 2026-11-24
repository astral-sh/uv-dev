# Arbitrary external directory deletion during PEP 832 .venv redirect replacement

Issue: astral-sh/uv#23340

## Summary

The reporter says `uv sync` on uv 0.13.9 rebuilt an incompatible virtual environment outside the project and left the project’s `.venv` redirect in place. Files in the external environment reportedly disappeared. The reporter expects uv to replace only the project-local redirect and leave the external directory untouched; whether uv should replace the redirect is disputed. The report lists Windows 11 and Linux (both x86_64), with Python 3.12.16 and 3.11.18. It separately reports an error on an empty redirect without enabling `venv-redirect-files`.

## Classification

Classification: bug. In the November 24, 2026 discussion, uv maintainer `zsol` identified the empty-file behavior as a bug. The checkout shows a controlled `InvalidData` error, not a crash: `resolve_project_env` propagates the error from `read_venv_redirect` before checking `venv-redirect-files`, which currently only silences the warning (`crates/uv-fs/src/lib.rs:973`, `crates/uv-environment-operations/src/lib.rs:448`, `crates/uv-preview/src/lib.rs:390`). The comment does not specify what uv should do with an empty `.venv` instead.

The external-target deletion remains a data-loss concern, but its classification is disputed. `zsol` considers the behavior likely intentional and argues that replacing the redirect would disregard the user's chosen environment location. This is a maintainer assessment, not a final decision that deletion is safe or that the report should be closed. The non-cache test expects target recreation and redirect retention; the introducing PR's description points the other way. Resolve the intended replacement policy separately from the empty-file bug.

## Reproduction

**Outcome: needs_more_information (corroborated by a second user, not verified locally).** A commenter separate from the original reporter says that `uv sync --python 3.12` on uv 0.13.9 replaced an external Python 3.11 environment, deleted their canary file, and left `.venv` as a redirect file. The commenter did not specify a platform or report on the empty-file error. The discussion does not establish `--reinstall` alone as a trigger.

The installed binary reports uv 0.13.7 (`x86_64-unknown-linux-gnu`) and the local Python is 3.12.3 on x86_64 Linux. No deletion scenario was run locally; this is not a negative reproduction. The remaining gap is controlled runtime confirmation on the reported uv 0.13.9 build, not an additional fixture from the reporter. The decisive observations are whether unrelated files in the external target survive and whether the project-local redirect is replaced.

- `crates/uv/tests/sync/centralized_project_envs.rs:677`, `sync_venv_redirect_file`: the checked-in snapshot expects an external environment to be removed and rebuilt after an interpreter change. The assertions check that the redirect contents are unchanged, the sentinel file `external/keep` is gone, and the target runs the newly selected interpreter. This test was inspected, not run.
- `crates/uv/tests/sync/centralized_project_envs.rs:636`, `sync_replaces_cached_venv_redirect_without_removing_target`: a distinct case checks that an incompatible redirect to a uv-managed cached environment is replaced with a local environment while the cached environment still runs its original interpreter. It does not cover preservation of a non-cache target. This test was also inspected, not run.

## Related

- astral-sh/uv#21822 introduced PEP 832 redirect handling. Its description says an incompatible redirect is replaced with a directory, but its non-cache test expects the target to be rebuilt and the redirect retained. This distinction matters when resolving the maintainer's concern about honoring the user's chosen environment location.
- astral-sh/uv#21364 reported earlier data loss through active-script environment selection, a different path from project redirect replacement.
- astral-sh/uv#21464 fixed astral-sh/uv#21364 by rejecting nonempty destinations that are not virtual environments for active scripts. That guard does not address replacement of a recognized external virtual environment selected by a redirect.
- astral-sh/uv#21487 proposes moving the non-virtualenv guard into the shared removal helper. It does not address whether a project redirect or its recognized target should be replaced.
