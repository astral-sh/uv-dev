# Feature Request: Support environment markers / conditions for `include-group` in Dependency Groups

Issue: astral-sh/uv#22266

Classification: enhancement

## Summary

The reporter requests Python-version and platform markers on individual `include-group` entries. Their example says `libtorrent~=2.1` does not support Python 3.14+: `uv sync --only-group libtorrent` should fail there, while `uv sync --group all` should omit the included `libtorrent` group and install the remaining dependencies. They propose either a marker appended to the group name or a separate `marker` field. The package compatibility claim was not independently reproduced; the requested configuration capability is clear from the report.

The closest precedents are astral-sh/uv#11606 and merged astral-sh/uv#13735 for mandatory group-level Python constraints. astral-sh/uv#15889 concerns explicit-selection errors in uv pip compile. None tracks conditional include-group entries.

## Draft response

Your distinction between explicitly selecting a group and conditionally including it isn't supported by group-level `requires-python` from astral-sh/uv#11606 and astral-sh/uv#13735: those constraints also apply to groups that include it. Adding markers to individual requirements would skip them even when the group is selected directly, so that doesn't preserve your requested behavior.

Because `include-group` is standardized by PEP 735, either proposed syntax would need to go through the Python standards process. A uv-specific metadata design is a possible alternative. The next step is to discuss that design here, including direct-selection versus inclusion semantics, before an implementation.

## Classification

The request adds environment-dependent include-group behavior, including different outcomes for direct and indirect group selection. Current parsing supports only a group name, and existing group requires-python constraints propagate to parent groups. The maintainer confirms that the proposed standardized syntax requires the Python standards process, while uv-specific metadata is a possible design direction. The historical group-constraint feature does not implement this capability; no duplicate or regression was established.

The relevant distinction is a condition on the inclusion of a group, rather than a requirement attached to the group itself. Per-dependency markers also change direct selection, so they are only a partial workaround. A concrete next step is design discussion of uv-specific metadata, or a packaging standards proposal for either syntax shown in the report.

## Related

- astral-sh/uv#11606 — Allow dependency groups to define a separate `requires-python` (issue, closed). Tracks group-level Python requirements and explicitly discusses failing when an incompatible group is selected. It was implemented by astral-sh/uv#13735. Unlike this request, it does not provide conditions on individual include-group entries or skip incompatible nested groups.
- astral-sh/uv#13735 — Add `[tool.uv.dependency-groups].mygroup.requires-python` (pull request, merged). Implemented astral-sh/uv#11606, merged June 13, 2025, and released in uv 0.7.14. Its review discussion explicitly explains that included groups' Python constraints are intersected for interpreter validation, preventing partial installation of a parent group. This establishes why existing group metadata does not satisfy conditional inclusion.
- astral-sh/uv#15889 — Group `requires-python` silently drops groups with `uv pip compile --group` (issue, open). Concerns silently omitting an explicitly selected, Python-incompatible group; a maintainer says this should error as project commands do. It supports the reporter's distinction between explicit selection and conditional inclusion, but concerns uv pip compile diagnostics rather than markers on nested groups.

## Supporting evidence

- The October 7, 2026 maintainer comment on astral-sh/uv#22266 states that the proposed syntax would require the Python standards process because the field is standardized, and identifies uv-specific metadata as a plausible alternative. This is a design possibility, not an implementation commitment.
- `crates/uv-pypi-types/src/dependency_groups.rs:88` defines `IncludeGroup` with only a `GroupName`. Its deserializer at line 134 accepts a single-key include map; other maps become unsupported dependency objects. Neither proposed syntax is currently supported.
- `crates/uv-workspace/src/dependency_groups.rs:144` copies included requirements and intersects the included group's `requires-python` into the parent group's constraints. Lines 168 onward apply group requirements and retain the intersection for interpreter discovery and checking. These are mandatory constraints, not conditions on inclusion.
- In the review discussion on astral-sh/uv#13735, Gankra explicitly explains that per-requirement markers could theoretically allow partial group installation, but interpreter validation rejects it using the intersected group constraints. Maintainers also confirm that selecting an incompatible group should produce an error.
- `docs/concepts/projects/dependencies.md:716` documents group nesting, and line 760 documents `[tool.uv.dependency-groups]` Python requirements. `crates/uv-workspace/src/pyproject.rs:915` exposes only `requires_python` in group settings.
- `changelogs/0.7.x.md:616` places astral-sh/uv#13735 in uv 0.7.14. The requested conditional inclusion was not part of that implementation, and the report does not claim that previously working support regressed.

## Search scope and exclusions

Used authenticated gh to search open and closed issues and open, closed, and merged PRs. Literal searches covered include-group with marker, conditional, python_version, and requires-python; platform_system; and libtorrent. Conceptual searches covered dependency-group environment markers, platform-specific groups, conditional groups, and skipping Python-incompatible groups, without package/version restrictions. Historical searches followed group requires-python discussions to astral-sh/uv#11606 and its implementing PR astral-sh/uv#13735; inspected comments, review threads, current source, and the 0.7.14 changelog. PR keyword searches returned no candidates even for the known implementation, so that PR was found through the issue timeline and inspected directly. Ruled out astral-sh/uv#11232 (propagating declared group conflicts), astral-sh/uv#15619 and astral-sh/uv#15520 (PEP 751 marker variables in unsupported contexts), and astral-sh/uv#9806 and astral-sh/uv#11474 (group inheritance into project dependencies). No matching conditional-inclusion discussion was found.

Also inspected astral-sh/uv#10200, an earlier discussion of stricter development-tool Python requirements, and astral-sh/uv#18221, which asks for dependency exclusions conditioned on selected groups. Neither requests environment markers on an inclusion. The standards-related inheritance discussions provide context, but do not centralize this request.

## Validation

This handoff is based on read-only GitHub inspection and checkout source/documentation review. No package installation or behavioral reproduction was needed to establish the missing feature. No checkout files or GitHub objects were changed.
