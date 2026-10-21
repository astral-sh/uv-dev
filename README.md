# `uv workspace metadata` and `uv check` fail with conflicting extras

Issue: astral-sh/uv#22796

Classification: bug

## Summary

No direct duplicate found. astral-sh/uv#20643 documents the relevant no-conflicts assumption; astral-sh/uv#22395 explains the current inspection path and error wording. astral-sh/uv#14848 concerns a distinct extras-selection problem in uv sync.

The reporter uses uv 0.13.4 on Ubuntu 24.04 x86_64. A single project declares extra `a` with `idna==3.9` and extra `b` with `idna==3.10`, and explicitly marks the extras as conflicting. After successfully syncing only extra `a`, workspace metadata exits with status 2. Its error says that both extras are enabled simultaneously. Running `uv check --extra a` then emits a `warning[uv-metadata]` diagnostic and exits with status 1.

The report supplies a complete minimal configuration. The issue was open with no comments when inspected.

## Report and expected behavior

The two observable symptoms are:

- `uv workspace metadata --preview-features workspace-metadata` fails while inspecting an existing environment, despite the preceding single-extra sync.
- `uv check --preview-features=check-command --extra a` surfaces the same metadata error through ty. The separate “No python files found” warning does not explain the conflicting-extras error.

The trigger is an existing environment plus mutually exclusive optional dependencies declared in `tool.uv.conflicts`. Neither failing command requests all extras. Expected behavior is to inspect the valid environment and provide metadata without treating both alternatives as simultaneously selected.

The exact identifiers and error fragments used for comparison were `workspace-metadata`, `check-command`, `warning[uv-metadata]`, `Failed to inspect environment`, and `Found conflicting extras`. The package name, pinned versions, and Linux distribution are incidental to the source-confirmed selection behavior.

Minimal configuration:

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

Reported command sequence:

```console
uv sync --quiet --extra a
uv workspace metadata --preview-features workspace-metadata
uv check --preview-features=check-command --extra a
```

## Draft response

Your conflict declaration is valid. Metadata inspection currently selects all extras, even when the environment was synced with only `a`, and `uv check` surfaces that failure through ty. The existing-environment integration in astral-sh/uv#20643 explicitly assumed no conflicts. This needs a fix in metadata’s environment inspection; your example is sufficient for regression tests covering both commands.

## Classification

Declared conflicting extras are supported when only one is selected. Source confirms that metadata inspection instead selects all extras and invokes conflict validation even without --sync, rejecting the valid environment and causing the downstream uv check diagnostic. Earlier metadata work acknowledged a no-conflicts assumption but did not fix this case. No existing issue tracks the same failure, and no previously fixed bug was found to have regressed.

The earlier assumption that metadata consumers would not encounter conflicts is a limitation of the implementation, not evidence that the reported behavior is correct. The merged feature PRs below provide implementation history; they do not already track a fix for this failure. The report does not establish a last working version, and the inspected history does not establish recurrence of a previously fixed bug.

## Related

- astral-sh/uv#20643 — Include best-effort `environment` info by default in `uv workspace metadata` (pull request, merged). Added inspection of an existing environment without --sync, specifically so uv check could retain its selected environment. The discussion explicitly assumed there were no conflicts. Its diff still computes package IDs using all extras before deciding whether to sync, directly explaining the unsupported case; it did not fix conflicting-extra inspection.
- astral-sh/uv#22395 — Include installed packages in workspace metadata (pull request, merged). Updated the failing environment-inspection path and introduced the reported 'Failed to inspect environment' context. Released in uv 0.13.2, before the reported 0.13.4. Its diff retains the pre-existing all-extras selection, so it is relevant implementation history, not evidence of a fix or a newly introduced conflict-selection bug.
- astral-sh/uv#14848 — incorrect extras conflict detection when using `uv sync` with `--package` and `--all-extras` (issue, open). Also concerns selecting conflicting extras beyond the intended installation. Maintainer comments discuss restricting --all-extras to selected packages. Unlike this report, it requires explicit --all-extras and --package during sync; it does not cover metadata inspection failing after a valid single-extra sync.

## Supporting evidence

Source inspection was performed at checkout commit `dce03cb`.

- `docs/concepts/resolution.md:612` documents explicitly conflicting extras as supported: they can be locked together but cannot be installed together.
- `docs/reference/internals/metadata.md:67` explains that workspace metadata can describe multiple package versions arising from conflicts and exposes those conflicts in its output. A conflict declaration is therefore not inherently invalid metadata input.
- `crates/uv-workspace-commands/src/metadata.rs:277` discovers an existing environment when sync is not requested; line 290 enters inspection whenever one exists. Line 312 adds the exact reported error context.
- `crates/uv-workspace-commands/src/environment.rs:56` calls `target_selection` and `selected_package_ids` before the optional synchronization branch. At line 216, project and workspace targets unconditionally select all extras and all dependency groups.
- `crates/uv-workspace-commands/src/environment.rs:124` passes those selections to `target.to_resolution`. `crates/uv-lock/src/lock/installable.rs:741` rejects activated extras belonging to the same conflict set. This establishes the metadata failure mechanism without requiring a runtime reproduction.
- `crates/uv-project-commands/src/check.rs:678` synchronizes using the requested extras. `crates/uv-project-commands/src/check/ty.rs:227` enables ty’s uv integration, and line 239 explicitly describes ty invoking workspace metadata. The reported diagnostic identifies that subprocess as the failing operation.
- astral-sh/uv#20501 (merged July 20, 2026) changed `uv check` to have ty call uv for metadata and documented limitations in replaying command-line selections. This supports the propagation path, rather than identifying a separate type-checking error.
- astral-sh/uv#20643 (merged July 23, 2026) extended collection to an existing environment without syncing. Its discussion explicitly describes the assumption that there are no conflicts. Its diff leaves all-extras package selection before the optional sync branch.
- astral-sh/uv#19122 introduced module ownership collection with all extras and dependency groups. Its discussion and review comments address installed-package metadata and inspection semantics, but contain no fix for this conflict case.
- astral-sh/uv#22395 (merged October 15, 2026) renamed the collection code and changed the outer error from `Failed to collect module owners` to `Failed to inspect environment`. Its diff retains the all-extras selection. `CHANGELOG.md:47` places this change in uv 0.13.2, released October 19, before the reported uv 0.13.4 release on October 20. The new wording alone does not establish a new regression.
- Existing tests in `crates/uv/tests/workspace/workspace_metadata.rs` cover inspection of an existing environment, installed-package inventory, and module-inspection failures. No explicit declared-conflicting-extras scenario was found in that module.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separately searched workspace metadata failures and uv check propagation using the exact errors, the historical 'Failed to collect module owners' wording, workspace-metadata, check-command, uv-metadata, extras, optional dependencies, mutually exclusive dependencies, all-extras, environment inspection, and module ownership, without package/version/platform restrictions. Supplemented empty PR text searches by filtering the newest 4,000 all-state PRs and inspected candidate bodies, comments, review threads, referenced PRs, diffs, and release notes. Ruled out astral-sh/uv#22673 (conflict-validation order), astral-sh/uv#22638 (metadata schema change), and astral-sh/uv#22046 (lockfile revalidation). No matching duplicate or previous fix was found.

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

Use the supplied example to cover metadata inspection of a valid single-extra environment and the downstream `uv check` invocation. The fix should make inspection respect declared conflicts without manufacturing a simultaneous selection of every extra. The exact selection and installed-package matching design remains implementation work.

Regression coverage belongs in the existing workspace metadata and project check test modules. Follow their snapshot conventions, inspect existing coverage before adding cases, and use the appropriate feature gate if a test downloads packages or ty.

This handoff is based on the report, source inspection, documentation, PR diffs, comments, and release notes. No runtime reproduction or tests were run. No checkout files or GitHub objects were modified; only this handoff README was authored.
