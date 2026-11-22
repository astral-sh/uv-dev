# Arbitrary external directory deletion during PEP 832 .venv redirect replacement

Issue: astral-sh/uv#23340

## Summary

The reporter says `uv sync` on uv 0.13.9 rebuilt an incompatible virtual environment outside the project and left the project’s `.venv` redirect in place. Files in the external environment reportedly disappeared. The expected outcome is to replace only the project-local redirect and leave the external directory untouched. The report lists Windows 11 and Linux (both x86_64), with Python 3.12.16 and 3.11.18. It separately reports an error on an empty redirect without opting into the preview warning.

## Classification

Security-sensitive bug report: potential data loss outside the project directory. The checkout’s tests distinguish external targets from uv-managed cached targets and make the reported outcome plausible, but reading those tests does not establish what happened on the reporter’s platforms or in the shipped binary. The empty-file report is separate: a controlled validation error is not a crash and does not by itself establish a broken preview gate.

## Reproduction

**Outcome: needs_more_information (not independently verified).** The installed binary reports uv 0.13.7 (`x86_64-unknown-linux-gnu`) and the local Python is 3.12.3 on x86_64 Linux. No deletion scenario was run; this is not a negative reproduction. The missing evidence is controlled runtime confirmation on the reported uv 0.13.9 build, not an additional fixture from the reporter. For an authorized verification, the decisive observations are whether unrelated files in the external target survive and whether the project-local redirect is replaced.

- `crates/uv/tests/sync/centralized_project_envs.rs:677`, `sync_venv_redirect_file`: the checked-in snapshot expects an external environment to be removed and rebuilt after an interpreter change. The assertions check that the redirect contents are unchanged and that the target runs the newly selected interpreter. This test was inspected, not run; it does not assert the fate of unrelated files.
- `crates/uv/tests/sync/centralized_project_envs.rs:636`, `sync_replaces_cached_venv_redirect_without_removing_target`: a distinct case checks that an incompatible redirect to a uv-managed cached environment is replaced with a local environment while the cached environment still runs its original interpreter. It does not cover preservation of a non-cache target. This test was also inspected, not run.

## Related

- astral-sh/uv#21822 introduced PEP 832 redirect handling. The prior triage notes describe intended redirect replacement and point to the differing behavior of the external-target test.
- astral-sh/uv#21364 reported earlier data loss through active-script environment selection, a different path from project redirect replacement.
- astral-sh/uv#21464 fixed astral-sh/uv#21364 by rejecting nonempty destinations that are not virtual environments for active scripts. That guard does not address replacement of a recognized external virtual environment selected by a redirect.
- astral-sh/uv#21487 proposes moving the non-virtualenv guard into the shared removal helper. It does not address whether a project redirect or its recognized target should be replaced.
