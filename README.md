# `uv workspace metadata` and `uv check` fail with conflicting extras

Issue: astral-sh/uv#22796

Classification: bug

## Summary

Both reported failures were reproduced with the supplied minimal configuration using the installed uv 0.13.4 on Ubuntu 24.04.5 LTS x86_64 and CPython 3.12.3. Syncing only extra `a` succeeds and installs `idna==3.9`. Workspace metadata then exits 2 with the reported conflicting-extras error, and `uv check --extra a` exits 1 with the same `warning[uv-metadata]` diagnostic. A separate `uv pip check` succeeds for the environment.

The report supplies a complete minimal configuration. It does not specify a Python version or a last working uv version; no upgrade regression is established. The issue was open with no comments when the earlier issue-context review was performed.

The earlier related-work search found no direct duplicate. astral-sh/uv#20643 documents the relevant no-conflicts assumption; astral-sh/uv#22395 explains the inspection path and error wording. astral-sh/uv#14848 concerns a distinct extras-selection problem in `uv sync`.

## Report and expected behavior

The project explicitly declares two mutually exclusive extras and requests only one during synchronization and checking. Expected behavior is for metadata inspection to describe the valid environment without treating both alternatives as selected, and for `uv check --extra a` to avoid a metadata conflict diagnostic.

Observed behavior matches the report: metadata fails while inspecting the existing environment, and ty propagates that failure through `uv check`. The separate “No python files found” warning also appears in the reproduction, but the diagnostic explicitly identifies the failed metadata subprocess and conflicting extras.

The documentation supports the bug classification independently of reproducing the error: `docs/concepts/resolution.md` permits declaring conflicting extras and installing one alternative, while `docs/reference/internals/metadata.md` describes metadata graphs containing multiple versions arising from declared conflicts. Neither failing command in this fixture requests all extras. No cross-platform or package-name/version variants were tested.

## Reproduction

**Outcome: reproducible.** Both commands produced the reported errors and exit codes on 2026-10-21.

### Environment and isolation

- OS: Ubuntu 24.04.5 LTS, Linux x86_64, matching the reported Ubuntu 24.04 x86_64 platform.
- Executable from `PATH`: `/opt/hostedtoolcache/uv/0.13.4/x86_64/uv`; version output: `uv 0.13.4 (x86_64-unknown-linux-gnu)`.
- Python: CPython 3.12.3, selected explicitly through `UV_PYTHON`. The report leaves Python unspecified.
- `uv check` downloaded and used ty 0.0.90.
- Project: `/tmp/uv-issue-22796-8trjppae/project`. All reproduction files, virtual environments, downloaded tools, and caches are under `/tmp/uv-issue-22796-8trjppae`.
- Commands ran with an allowlisted environment, temporary `UV_CACHE_DIR`, `UV_CREDENTIALS_DIR`, `UV_PYTHON_INSTALL_DIR`, `UV_TOOL_DIR`, `UV_TOOL_BIN_DIR`, and `TMPDIR`. `UV_NO_CONFIG=1` avoided external uv configuration; `NETRC` pointed to a new empty temporary file and keyring integration was disabled. Python downloads and source builds were disabled. The fixture used public PyPI wheels.

### Minimal fixture and commands

Create this `pyproject.toml` in a new temporary project directory:

```toml
[project]
name = "mre"
version = "0.1.0"
requires-python = ">=3.11"

[project.optional-dependencies]
a = ["idna==3.9"]
b = ["idna==3.10"]

[tool.uv]
conflicts = [[{ extra = "a" }, { extra = "b" }]]
```

Run from that directory:

```console
uv sync --quiet --extra a
uv workspace metadata --preview-features workspace-metadata
uv check --preview-features=check-command --extra a
```

There are no workspace members, dependency groups, Python source files, or pre-existing lockfile or environment. The commands use the default non-frozen behavior. The configuration is the complete reported fixture, reconstructed independently; no variants were needed.

### Observed results

1. `uv sync --quiet --extra a` exits **0**, with empty stdout and stderr.
2. `uv workspace metadata --preview-features workspace-metadata` exits **2**, with empty stdout and this stderr:

   ```text
   Resolved 3 packages in 0.51ms
   error: Failed to inspect environment
     cause: Found conflicting extras `mre[a]` and `mre[b]` enabled simultaneously
   ```

3. `uv check --preview-features=check-command --extra a` exits **1** and prints this diagnostic to stdout:

   ```text
   warning[uv-metadata]: `uv workspace metadata` failed with `exit status: 2`
   --> pyproject.toml:1:1
   info: Failed to inspect environment
     cause: Found conflicting extras `mre[a]` and `mre[b]` enabled simultaneously

   Found 1 diagnostic
   ```

   Stderr records the ty 0.0.90 download and `WARN No python files found under the given path(s)`.

Inspection through `.venv/bin/python` confirms Python 3.12.3 and `idna==3.9`. The additional command `uv pip check --python .venv/bin/python` exits **0** and reports `Checked 1 package` and `All installed packages are compatible`. Thus the environment has the requested alternative installed; the reported failure does not require installing both versions.

Raw exit codes and stdout/stderr are retained in `sync.json`, `metadata.json`, `check.json`, and `pip-check.json` under the temporary reproduction root.

### Existing test coverage

Reviewed setup and assertions in the relevant integration tests; no declared-conflicting-extras case was found in `crates/uv/tests/workspace/workspace_metadata.rs` or `crates/uv/tests/project/check.rs`.

- `crates/uv/tests/workspace/workspace_metadata.rs::workspace_metadata_includes_existing_environment` locks two locally generated wheel dependencies, installs only one, deletes the uninstalled wheel, and verifies that frozen metadata inspection succeeds without syncing. It checks installed-package and module-owner output, lockfile-only discovery, and an overridden environment. It has no extras or conflict declaration.
- `crates/uv/tests/workspace/workspace_metadata.rs::workspace_metadata_module_owners_failure_is_error` deletes a locked wheel and snapshots exit 2 with `Failed to inspect environment` under `--frozen --sync`. Its failure is a missing distribution during synchronization, not conflicting-extra inspection.
- `crates/uv/tests/project/check.rs::check_no_sync_frozen_uses_existing_lock_without_update` makes manifest requirements unresolvable after locking, then verifies frozen checking retains the lockfile and does not install the dependency. Its exit-1 `warning[uv-metadata]` is about missing module ownership/editable paths, not conflicting extras.

No repository tests were added or executed. A regression test using public PyPI dependencies needs `test-pypi` gating; the project check module already requires both `test-python` and `test-r2`. Local wheel fixtures are available in the metadata module. Any added coverage should use the existing snapshot style and separate named tests for the two commands.

## Classification

Bug; runtime reproduction confirms both reported symptoms. Explicitly conflicting extras can be locked together and used one at a time. The successful single-extra sync and compatibility check demonstrate a valid installed environment, while metadata inspection rejects it as if both extras were enabled.

Source inspection is consistent with the observed failure: the metadata environment-inspection path selects all extras and groups, then constructs a selected resolution before deciding whether to synchronize. Conflict validation can therefore reject the inspection even without `--sync`. Earlier metadata work acknowledged a no-conflicts assumption. This is an implementation limitation rather than an invalid conflict declaration.

The related merged feature PRs provide implementation history; they do not track a fix for this failure. The report supplies no last working version, and this reproduction tested only uv 0.13.4. It establishes neither an upgrade regression nor the first affected release.

## Related

- astral-sh/uv#20643 — Include best-effort `environment` info by default in `uv workspace metadata` (pull request, merged). Added inspection of an existing environment without --sync, specifically so uv check could retain its selected environment. The discussion explicitly assumed there were no conflicts. Its diff still computes package IDs using all extras before deciding whether to sync, directly explaining the unsupported case; it did not fix conflicting-extra inspection.
- astral-sh/uv#22395 — Include installed packages in workspace metadata (pull request, merged). Updated the failing environment-inspection path and introduced the reported 'Failed to inspect environment' context. Released in uv 0.13.2, before the reported 0.13.4. Its diff retains the pre-existing all-extras selection, so it is relevant implementation history, not evidence of a fix or a newly introduced conflict-selection bug.
- astral-sh/uv#14848 — incorrect extras conflict detection when using `uv sync` with `--package` and `--all-extras` (issue, open). Also concerns selecting conflicting extras beyond the intended installation. Maintainer comments discuss restricting --all-extras to selected packages. Unlike this report, it requires explicit --all-extras and --package during sync; it does not cover metadata inspection failing after a valid single-extra sync.

## Supporting evidence

Source inspection was performed at checkout commit `dce03cb`. The runtime observations above used the installed uv 0.13.4 executable, not a binary built from this checkout.

- `docs/concepts/resolution.md:612` documents explicitly conflicting extras as supported: they can be locked together but cannot be installed together.
- `docs/reference/internals/metadata.md:67` explains that workspace metadata can describe multiple package versions arising from conflicts and exposes those conflicts in its output. A conflict declaration is therefore not inherently invalid metadata input.
- `crates/uv-workspace-commands/src/metadata.rs:277` discovers an existing environment when sync is not requested; line 290 enters inspection whenever one exists. Line 312 adds the exact reported error context.
- `crates/uv-workspace-commands/src/environment.rs:56` calls `target_selection` and `selected_package_ids` before the optional synchronization branch. At line 216, project and workspace targets unconditionally select all extras and all dependency groups.
- `crates/uv-workspace-commands/src/environment.rs:124` passes those selections to `target.to_resolution`. `crates/uv-lock/src/lock/installable.rs:741` rejects activated extras belonging to the same conflict set. This path is consistent with the conflict reported by the runtime reproduction.
- `crates/uv-project-commands/src/check.rs:678` synchronizes using the requested extras. `crates/uv-project-commands/src/check/ty.rs:227` enables ty’s uv integration, and line 239 explicitly describes ty invoking workspace metadata. The reported diagnostic identifies that subprocess as the failing operation.
- astral-sh/uv#20501 (merged July 20, 2026) changed `uv check` to have ty call uv for metadata and documented limitations in replaying command-line selections. This supports the propagation path, rather than identifying a separate type-checking error.
- astral-sh/uv#20643 (merged July 23, 2026) extended collection to an existing environment without syncing. Its discussion explicitly describes the assumption that there are no conflicts. Its diff leaves all-extras package selection before the optional sync branch.
- astral-sh/uv#19122 introduced module ownership collection with all extras and dependency groups. Its discussion and review comments address installed-package metadata and inspection semantics, but contain no fix for this conflict case.
- astral-sh/uv#22395 (merged October 15, 2026) renamed the collection code and changed the outer error from `Failed to collect module owners` to `Failed to inspect environment`. Its diff retains the all-extras selection. `CHANGELOG.md:47` places this change in uv 0.13.2, released October 19, before the reported uv 0.13.4 release on October 20. The new wording alone does not establish a new regression.
- Existing tests in `crates/uv/tests/workspace/workspace_metadata.rs` cover inspection of an existing environment, installed-package inventory, and module-inspection failures. No explicit declared-conflicting-extras scenario was found in that module.

## Search scope and excluded candidates

The earlier issue-context review searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separately searched workspace metadata failures and uv check propagation using the exact errors, the historical 'Failed to collect module owners' wording, workspace-metadata, check-command, uv-metadata, extras, optional dependencies, mutually exclusive dependencies, all-extras, environment inspection, and module ownership, without package/version/platform restrictions. Supplemented empty PR text searches by filtering the newest 4,000 all-state PRs and inspected candidate bodies, comments, review threads, referenced PRs, diffs, and release notes. Ruled out astral-sh/uv#22673 (conflict-validation order), astral-sh/uv#22638 (metadata schema change), and astral-sh/uv#22046 (lockfile revalidation). No matching duplicate or previous fix was found.

Searches covered both reported commands separately before investigating their shared implementation. Repository vocabulary included the `area:uv-check` and `area:workspaces` labels and the concepts of selected resolutions, installed distributions, module ownership, and conflict markers. Historical-error searches account for the changed outer error wording.

The all-state PR listing was bounded to the newest 4,000 PRs; it reached the original dependency-metadata and module-ownership implementations. Candidate inspection included both issue comments and relevant PR review comments.

Additional plausible candidates were excluded:

- astral-sh/uv#22673 changes when genuinely conflicting extras are rejected during dependency activation; it does not change metadata’s all-extras selection.
- astral-sh/uv#22638 changes the shape of the module-ownership JSON and depends on a corresponding ty schema update; it does not address conflict handling.
- astral-sh/uv#22046 and its fixing PR astral-sh/uv#22055 concern unnecessary lockfile re-resolution with `lock-without-metadata`, not inspection of an existing environment.
- astral-sh/uv#21720 concerns Python-version-dependent differences between `uv check` and `ty check`; maintainer comments confirm different interpreter-version preferences.
- astral-sh/uv#19790 concerns a system environment without `pyvenv.cfg`, not conflicting extras.
- astral-sh/uv#11648 concerns installing multiple versions simultaneously; here the reported single-extra sync succeeds and inspection subsequently rejects an artificial all-extras selection.
- astral-sh/uv#21397 requests ownership information for packages absent from the lockfile. It led to relevant inspection history but is a different capability request.
- astral-sh/uv#12541 requested the metadata command itself; its closing maintainer comment directs specific follow-up requests to new issues.

## Recommended follow-up and validation limits

Add regression coverage for metadata inspection of a valid single-extra environment and the downstream `uv check` invocation in the existing test modules. Inspection should respect declared conflicts without manufacturing a simultaneous selection of every extra. The exact selection and installed-package matching design remains implementation work.

Both reported failures are now supported by runtime evidence. The historical GitHub search findings above are retained from the earlier context review; this reproduction did not repeat those searches. No other uv versions or platforms were tested, and no repository test suite or build was run. No checkout files, existing user state, or GitHub objects were changed. The only handoff document updated was this README; reproduction artifacts remain in the temporary directory.
