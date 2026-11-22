# Arbitrary external directory deletion during PEP 832 .venv redirect replacement

Issue: astral-sh/uv#23340

Classification: bug

## Summary

In uv 0.13.9, `uv sync` can rebuild an incompatible virtual environment outside the project that `.venv` points to, deleting files in the target and leaving the redirect in place. The reporter expects uv to replace the local redirect without modifying the target. The report also describes an empty `.venv` file producing a redirect-specific error without the preview option.

astral-sh/uv#21822 introduced the redirect behavior and added a test that expects the non-cache target to be rebuilt. The earlier active-script deletion fix covers a different path. No separate issue or pull request was found that tracks either reported behavior.

## Draft response

We confirmed from the 0.13.9 source that `uv sync` can remove files in a non-cached environment when switching Python versions through a `.venv` redirect. Replacement should leave the target alone. Until this is fixed, move only the `.venv` redirect file aside and use a project-local environment before syncing with a different Python version. The empty-file error also occurs without `--preview-features venv-redirect-files`: that option silences the warning but does not turn redirect handling on or off.

## Classification

The source confirms a destructive replacement of the referenced environment instead of the project-local redirect. The description of astral-sh/uv#21822 says an incompatible redirect is replaced with a directory, while its tests explicitly expect non-cache targets to be rebuilt in place. That test documents what shipped; it does not make the target deletion safe. This is a bug introduced with the redirect feature, not a recurrence of the previously fixed active-script bug or a duplicate of the feature's implementation PR.

The empty-file behavior is a controlled error, not a crash. Empty redirects are deliberately rejected so they cannot resolve to the current directory. Since the preview setting controls only the warning, the reported error without the flag is real, but it does not establish a broken opt-in gate. The destructive replacement is sufficient for the bug classification.

## Related

- astral-sh/uv#21822 (merged): Introduced PEP 832 redirect handling in uv 0.13.9. Its description says an incompatible redirect is replaced with a directory, but its non-cache sync test expects target deletion and redirect retention. Its review also explains why empty redirects are rejected.
- astral-sh/uv#21364 (closed): Earlier data loss during environment replacement. `uv run --active --script` selected a non-virtualenv directory through `VIRTUAL_ENV`; the new report instead involves a recognized virtual environment selected by a project redirect file.
- astral-sh/uv#21464 (merged): Fixed astral-sh/uv#21364 by rejecting nonempty non-virtualenv destinations for active scripts. It does not protect a recognized virtualenv behind a project redirect.
- astral-sh/uv#21487 (open): Follow-up to put non-virtualenv protection in the shared removal helper. Its proposed guard still permits replacing recognized virtualenvs, so it does not address replacing the redirect rather than its target.

## Source evidence

- The changelog records redirect support in uv 0.13.9, released November 17, 2026 (`CHANGELOG.md:8`). Both the release tag and the checkout route a valid non-cache redirect into `ProjectEnvironmentRoot::Directory(target)` instead of retaining the redirect as the replacement path (`crates/uv-environment-operations/src/lib.rs:443`). When the existing environment is incompatible, the project path clears and recreates the resolved directory (`crates/uv-environment-operations/src/lib.rs:1048`, `crates/uv-environment-operations/src/lib.rs:1129`). The release calls `clear_virtualenv`, which in turn calls `remove_virtualenv` (`crates/uv-fs/src/lib.rs:1115`). It does not accept an unrelated directory as a redirect target merely because it exists: discovery checks that it is a virtualenv base.
- `sync_venv_redirect_file` asserts that the non-cache target's sentinel is gone and the redirect is unchanged after an interpreter switch. `sync_replaces_cached_venv_redirect_without_removing_target` asserts the opposite for a cache-backed target: the local redirect becomes a directory and the cached environment survives (`crates/uv/tests/sync/centralized_project_envs.rs:677`, `crates/uv/tests/sync/centralized_project_envs.rs:636`). Update the former expectation to assert that replacement keeps the target intact while retaining the cache-backed coverage.
- `read_venv_redirect` returns `InvalidData` for an empty file before the caller reaches the warning check (`crates/uv-fs/src/lib.rs:972`, `crates/uv-environment-operations/src/lib.rs:448`). The preview feature documentation says that enabling it silences the warning (`crates/uv-preview/src/lib.rs:390`). The existing invalid-redirect sync test covers a missing target, not an empty file (`crates/uv/tests/sync/centralized_project_envs.rs:791`).
- The interpreter mismatch is confirmed by source and the checked-in test, not by running the destructive scenario. `--reinstall` is documented as reinstalling packages and is not passed to project environment discovery; it is not established as a standalone trigger (`crates/uv-cli/src/lib.rs:7083`, `crates/uv-project-commands/src/sync.rs:214`).

## Search scope

Searched open and closed issues and open, closed, and merged pull requests through `gh`, checking recent PR listings and linked records in addition to search results. Literal queries used `PEP 832`, `venv-redirect-files`, `read_venv_redirect`, the empty-redirect diagnostic, and the report and introducing PR references. Symptom queries covered external environment deletion during sync or interpreter switching, unexpected environment recreation, and empty or non-directory `.venv` paths. Mechanism and fix queries covered centralized project environments, symlinks, `pyvenv.cfg`, and `remove_virtualenv`. Checked candidate discussions, cross-references, the introducing PR's review and tests, and the 0.13.9 release source.

Ruled out astral-sh/uv#10325, which asks for a general opt-out from recreation rather than preservation of a redirect's target, and astral-sh/uv#11850, which predates redirect support and did not establish a redirect-related cause for repeated recreation. The symlink cases are distinct: astral-sh/uv#14670 and astral-sh/uv#14933 concern retaining a filesystem symlink in `uv venv`; astral-sh/uv#23055 and astral-sh/uv#23158 concern writes through symlinked subdirectories during `uv venv --allow-existing`; astral-sh/uv#23319 and astral-sh/uv#23325 concern writes through `.python-version` and `uv.lock` symlinks. For the empty-file symptom, astral-sh/uv#9423 concerns a non-virtualenv directory, and astral-sh/uv#22950 and astral-sh/uv#23339 concern parsing the preview configuration itself.
