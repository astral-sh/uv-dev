# Expose module ownership for installed packages outside the lockfile

Issue: astral-sh/uv#21397

Classification: enhancement

## Summary

`uv workspace metadata` currently describes the locked resolution and, when an environment is inspected, emits `module_owners` only for installed distributions that can be mapped back to nodes in that resolution. The reported sequence—install two independent dependencies, remove one with `uv remove --no-sync`, and run ty with `missing-direct-dependency` enabled—leaves one distribution importable but absent from both `resolution` and `module_owners`. As a result, ty cannot identify the import as coming from an installed but undeclared distribution. An exact `uv sync` removes that distribution, after which ty can report the import as unresolved.

The request is to add lockfile-independent installed-state information to the experimental workspace metadata schema, including module ownership for installed-only distributions, while keeping that inventory separate from the locked dependency graph.

The closest implementation history is astral-sh/uv#19122, which introduced `module_owners` and deliberately restricted owners to the selected resolution. Its review discussion anticipated a separate environment section listing installed package IDs. The closest open issue is astral-sh/uv#10962, a broader request for machine-readable metadata for every installed distribution; it does not cover module ownership or the workspace metadata schema.

## Draft response

The source confirms the behavior you described: the current implementation intentionally omits these distributions. Module-owner collection assigns package IDs only to distributions in the selected resolution, and the metadata export filters owners against resolution nodes. That behavior came from astral-sh/uv#19122; its review also anticipated a separate environment section identifying installed package IDs. astral-sh/uv#10962 is related, but it requests broader installed-distribution metadata and does not cover module ownership or the workspace metadata schema.

This looks like a distinct enhancement to the preview workspace metadata schema. The next step is to agree on a representation for installed-only distributions and stable ownership IDs that remains separate from `resolution`, including how inherited environments and distributions with incomplete module records should be handled.

## Classification

This is an enhancement, not a duplicate or regression. Repository source and the existing integration test confirm that the omission is deliberate current behavior:

- `selected_package_ids` constructs ownership IDs only from distributions in the selected resolution.
- `find_module_owners_in_environment` skips every installed distribution whose name has no selected package ID.
- `Metadata::with_module_owners` performs a second filter requiring each owner ID to exist in `resolution`.
- `workspace_metadata_module_owners_ignore_stale_virtual_package` snapshots the resulting absence of `module_owners` for an installed distribution outside the project resolution.

The requested separate installed-environment inventory would therefore add schema and behavior to the preview command. astral-sh/uv#10962 overlaps at the level of installed-package inspection, but it asks for general package metadata through a pip-oriented inspection interface. astral-sh/uv#19122 is merged implementation history rather than an open tracker for this additional capability.

## Related

- astral-sh/uv#19122 — “Add module owners to workspace metadata” (merged). This pull request introduced the exact field involved. Its description says owners are filtered through the selected installed resolution, and its review considered a future split where a sync or virtual-environment section lists installed package IDs while package nodes carry optional module information.
- astral-sh/uv#10962 — “Retrieving metadata for installed packages” (open). This enhancement asks for programmatic metadata for every installed package and is the closest active installed-environment request, but its proposed interfaces and fields concern general distribution metadata rather than import-module ownership in `uv workspace metadata`.

## Search and supporting evidence

Literal searches covered `workspace metadata`, `module_owners`, `module owners`, `installed distributions`, `installed packages`, `outside the lockfile`, `missing-direct-dependency`, `remove --no-sync`, and `site-packages`. Conceptual searches covered extraneous or stale packages, environment inventory, installed state, ownership of imports, and locked versus installed resolution. Searches included open and closed issues and open, closed, and merged pull requests. Fix- and history-oriented inspection covered astral-sh/uv#19122 and its review thread; reference chains from astral-sh/uv#10962 covered astral-sh/uv#10886, astral-sh/uv#2526, and astral-sh/uv#7532.

astral-sh/uv#12541 was a plausible broader candidate because it requested cargo-like machine-readable project metadata. It was closed after `uv workspace metadata` became available, with maintainers asking for new issues for specific additions, so it does not track installed-only distributions or module ownership and is not a close related item. astral-sh/uv#10886, astral-sh/uv#2526, and astral-sh/uv#7532 focus on dependency listings, `uv pip show`, or a high-level package information command; none covers the reported workspace metadata gap.
