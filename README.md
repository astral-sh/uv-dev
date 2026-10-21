# PEP723 script using project extras or dependency group

Issue: astral-sh/uv#22792

Classification: duplicate

## Summary

The reporter wants PEP 723 scripts to provide environment and script management while reusing definitions from `pyproject.toml`. They ask how to install the local project with extras in editable mode and how to select existing project dependency groups. Their example proposes a `[tool.environment]` table with `name = "docs"`, `root = ".."`, and `dependency-groups = ["docs"]`, intended for multiple runners such as uv and Hatch. No failing command, error output, uv version, platform, or previously working version is supplied.

astral-sh/uv#11348 is the closest discussion for combining inline scripts with project dependencies; astral-sh/uv#11383 covers per-script group selection. astral-sh/uv#17244 and astral-sh/uv#13997 explain the script-local group limitation. astral-sh/uv#5903 provides broader task-runner context.

Editable local project sources already work with uv-specific inline configuration. Directly importing project dependency groups through the proposed metadata is not supported. The best place to centralize the remaining integration request is astral-sh/uv#11348, while retaining the reporter’s group-selection and interoperability requirements.

## Draft response

You can already use an editable local project with an extra through uv’s script source configuration. For an installable project named `my-project` one directory above the script, with a `docs` extra:

```python
# /// script
# dependencies = ["my-project[docs]"]
# [tool.uv.sources]
# my-project = { path = "..", editable = true }
# ///
```

The source configuration is uv-specific. It selects an extra, not a dependency group. Referencing project dependency groups through `[tool.environment]` is not currently supported. Declaring groups inside scripts is also unsupported; astral-sh/uv#17244 explains the standards limitation.

Please continue the project-dependency integration request in astral-sh/uv#11348, including your requirements for selecting groups and sharing metadata across runners. That discussion is still open and marked as not on the immediate roadmap.

## Classification

The unresolved capability is reusing project dependency definitions while running a PEP 723 script, already requested in astral-sh/uv#11348. Selecting particular groups and expressing the selection in portable inline metadata are additional design requirements that can be centralized there; astral-sh/uv#11383 also covers per-script optional-group selection. Editable local project dependencies and their extras already have a uv-specific mechanism. No incorrect behavior or recurrence of a previously fixed bug is established.

The duplicate recommendation rests on the common capability requested in astral-sh/uv#11348, not merely on shared terminology. Both reports seek to prepare an inline script’s environment from project dependency definitions instead of repeating them in every script. The earlier issue proposes a command-line opt-in and does not specify this report’s exact metadata syntax or group subset. Those are design additions to the same integration discussion.

The underlying request would otherwise be an enhancement. The supported editable-source syntax answers part of the report, but does not make the unresolved group-import request merely a support question. The standards discussion in astral-sh/uv#17244 concerns declaring groups inside a script, so it should not be represented as an exact duplicate or a categorical rejection of importing project groups. No candidate pull request was created in response to this newly opened issue.

## Related

- astral-sh/uv#11348 — Run script that has inline deps with project deps (issue, open). Closest existing request: allow a PEP 723 script to use project dependencies alongside inline dependencies, avoiding duplicated declarations. The proposed --with-project option differs from this report’s inline group-selection syntax, but the underlying project/script integration request can be discussed together. Labeled enhancement and wish.

- astral-sh/uv#11383 — Handle dependencies per script, in the `pyproject.toml` (issue, open). Requests automatic selection of a project optional dependency group for a particular script. Closely overlaps the per-script extras/group selection goal, but places the mapping in pyproject.toml and concerns project entry points rather than PEP 723 metadata.

- astral-sh/uv#17244 — Support devDependencies for uv run --script (issue, closed). Maintainer explains that PEP 735 covers dependency groups in pyproject.toml, not inline scripts, and prefers a standards solution. This concerns declaring development groups inside a script; the new report instead proposes referencing groups already defined in a project.

- astral-sh/uv#13997 — Do not allow `uv add --group ... --script` (pull request, merged). Merged June 12, 2025, fixing astral-sh/uv#13988 by rejecting group/dev additions to scripts and adding regression coverage. Confirms that writing script-local dependency groups is intentionally unsupported; it does not implement importing groups from pyproject.toml.

- astral-sh/uv#5903 — Using `uv run` as a task runner (issue, open). Reporter-linked task-runner proposal, labeled needs-design, with maintainer discussion of standards and environment workflows. Broader context only: named task definitions do not establish support for portable PEP 723 references to project extras or dependency groups.

## Supporting evidence

Evidence was inspected against checkout commit `79422c835f214875852bf2b2cb950c44684eb046` and GitHub state on October 21, 2026.

- `docs/guides/scripts.md:197` explicitly states that inline script metadata causes project dependencies to be ignored.
- `crates/uv-project-commands/src/run.rs:513` handles the script environment separately; the following branches warn that project extras and dependency-group flags are unsupported for inline scripts. This restriction on project-selection flags is distinct from requesting a dependency’s extra using a PEP 508 requirement.
- `crates/uv-scripts/src/lib.rs:361` models inline dependencies as PEP 508 requirements. `Tool` and `ToolUv` at lines 421–436 expose uv configuration and sources, with no implementation for the proposed `tool.environment` table.
- `crates/uv/tests/project/edit.rs:6911`, `add_script_relative_path`, demonstrates `uv add ./project --editable --script script.py` and snapshots the generated inline path source with `editable = true`.
- `crates/uv-project-commands/src/add.rs:897` adds requested extras before resolving either script or project requirements. Its script branch resolves sources relative to the script directory.
- `crates/uv/tests/lock/lock.rs:38278`, `lock_script_editable_path_dependency_change`, covers locking an editable local script dependency and responding to changed package metadata. Editable script sources are existing functionality.
- In astral-sh/uv#17244, charliermarsh’s January 4, 2026 comment explains that PEP 735 only covers `pyproject.toml`, declines adding a new reliance on the legacy dev-dependencies setting, and prefers resolving script-local groups through standards.
- The inspected diff for astral-sh/uv#13997 rejects `uv add --group ... --script` and `--dev --script`; its test establishes that rejection. It merged on June 12, 2025. This was a validation fix, not implementation of group imports.
- astral-sh/uv#11348 remains open with `enhancement` and `wish`; the latter means “Not on the immediate roadmap.” Its author describes repeated shared dependencies across inline scripts. astral-sh/uv#11383 remains open and requests automatically choosing an optional group for a script.

## Search scope and excluded candidates

Decomposed the report into editable local project extras, importing project dependency groups into PEP 723 environments, and portable task/environment metadata. Used authenticated gh searches across open and closed issues with PEP723/PEP 723, script extras/editable, dependency groups/dependency-groups, optional-dependencies, inline metadata, tool.environment, with-project, reuse, and inherit. No error, version, or platform was reported. PR keyword searches returned no matches, so supplemented them by retrieving all 13,194 open, closed, and merged PR titles and inspecting relevant bodies, comments, reviews, and diffs. Followed astral-sh/uv#13988 to astral-sh/uv#13997 and astral-sh/uv#18312 to astral-sh/uv#18328. Ruled out the latter pair’s fixed editable-lockfile invalidation bug; astral-sh/uv#21458 only changes environment reuse when dependencies are omitted; astral-sh/uv#20836 tests workspace group selection for ordinary scripts; astral-sh/uv#12529 requests copying metadata rather than runtime reuse. Compared candidates against current documentation, source, and existing tests.

Literal searches treated extras/editability and project-group reuse separately. Conceptual searches covered shared dependencies, optional groups, inline/project integration, inheritance, environment reuse, and task-runner design. The exact proposed identifier `tool.environment` produced only the new report. Fix-oriented review examined group/script validation and editable-script locking independently from feature requests. PR search results were supplemented with the complete title inventory because the search endpoint did not return PR matches.

Especially plausible candidates were distinguished as follows:

- astral-sh/uv#18312 and its merged fix astral-sh/uv#18328 concern stale script lockfiles after an editable dependency’s metadata changes. The new report supplies no stale-lockfile symptom, so there is no evidence of a regression of that March 2026 fix.
- astral-sh/uv#21458 is open and changes cached/active environment reuse when the script omits `dependencies`. Its body, diff, and review comments do not add project extras or dependency-group imports.
- astral-sh/uv#20836 is merged test coverage for selecting root/member dependency groups when running ordinary workspace scripts without PEP 723 metadata. It does not implement inline group references.
- astral-sh/uv#12529 requests copying project dependencies into script metadata to support development tooling. A maintainer says they do not plan to invest in duplicating those definitions. Copying metadata differs from the runtime reuse proposed here.
- astral-sh/uv#16587 concerns reading script source overrides via `--with-requirements`, the opposite direction from importing project groups into a script environment.
- astral-sh/uv#13095 concerns installed entry points across workspace members, and astral-sh/uv#8620 concerns consistent tool versions across isolated environments. Neither supplies a closer PEP 723 group-import discussion.

## Verification and handoff

The conclusions are based on repository documentation, implementation, existing tests, and inspected GitHub discussions and changes. No build or reproduction was needed for this capability request, and no tests were run. The proposed reply is for maintainer review only. No GitHub mutations or checkout edits were made. The checkout’s existing modified `agents/codex/config.toml` and untracked event file were left unchanged.
