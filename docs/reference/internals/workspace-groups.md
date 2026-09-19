# Workspace groups

Workspaces share a lockfile so that related projects can agree on dependency versions. A large
workspace can also contain alternative sets of applications that cannot all use the same versions or
support the same Python range. Workspace groups name those sets of resolution roots. All groups
share one lockfile, using shared versions where possible and automatically forking when their
requirements cannot be satisfied together. Each group must be independently resolvable.

## Configuration

```toml title="pyproject.toml"
[tool.uv.workspace]
members = ["packages/*"]

[[tool.uv.workspace.groups]]
name = "main"
members = ["api", "worker"]
requires-python = ">=3.12,<3.13"
default = true

[[tool.uv.workspace.groups]]
name = "python-next"
members = ["api", "next-worker"]
requires-python = ">=3.12,<3.15"
```

The workspace's `members` and `exclude` fields discover projects by path. A group's `members` refer
to the `project.name` values of already-discovered workspace members, not paths or globs. Membership
can overlap. A workspace root with a `[project]` table is not implicitly added to every group.

Names are unique and occupy a separate namespace from `[dependency-groups]`. Unknown members, empty
groups, duplicate group names, and multiple `default = true` groups are configuration errors.

`requires-python` is optional. It restricts the group's Python compatibility; it cannot make a
member support another Python version. The effective domain also includes the Python requirements of
local members reached through production dependencies. Dependency and source markers retain their
conditions, so a conditional local dependency does not restrict unrelated environments. An empty
domain is an error. If no Python requirement is declared, uv defaults to the interpreter's minor
version. Members outside the roots contribute dependencies only when reached. `--no-sources` also
excludes the Python restrictions of local members whose sources are disabled.

## Command selection

`uv lock` resolves every declared group into the same `uv.lock`, regardless of the default group.
`--workspace-group <name>` selects one group's roots and locked solution for `uv sync`, `uv run`,
and `uv export`:

```console
$ uv lock
$ uv sync --workspace-group main
$ uv export --workspace-group python-next --format requirements-txt
```

`default = true` supplies the group for an otherwise unqualified command. An explicit workspace
group takes precedence. `--group` still selects dependency groups. With a selected workspace group,
`--package` targets a package reachable in that group's resolution; `--all-packages` targets the
group's locked roots.

Without an explicit or default workspace group, commands retain their normal project and package
targeting. If several groups contain the target, their package choices must agree wherever their
environments overlap. Compatible contexts are combined; different choices for an overlapping
environment require an explicit `--workspace-group`. A target absent from every group must be added
to a group before it can be installed or exported. An implicit all-members solve is not required,
because it would reintroduce the incompatibilities the named groups separate.

Group selection works with `--frozen` and does not re-resolve the graph. Workspace groups do not
create additional virtual environments, change published package metadata, or change source and
index precedence. Selecting several named groups in one command is not supported.

## Resolution

The workspace retains all discovered members for source lookup, while a resolution view exposes only
the selected roots. Each root is guarded by the environment in which its group is active. The
workspace Python domain is the union of those environments, not the intersection of every discovered
member's declaration.

Locking first attempts a shared universal resolution of all groups. Ordinary resolver backtracking
runs before a failure is treated as evidence that contexts need to split. An incompatible batch is
divided and retried until each batch resolves. A failure in a single group remains an error: no
required member or dependency is removed to make the group solvable. Source and index conflicts use
the same splitting path as version conflicts. This also handles higher-order incompatibilities,
where every pair of groups could resolve together but the complete set cannot.

Existing group-specific lock choices are preferred on subsequent locks. The first successful result
also supplies version preferences to later contexts, helping unrelated dependencies remain shared.
The partition is deterministic in configuration order. Sharing is an objective, not a guarantee of a
globally minimum number of versions. The algorithm does not enumerate every subset or pair of
workspace members.

## Lockfile

An ordinary workspace continues to write lockfile format version 1. A grouped workspace writes
version 2 so older readers cannot silently install a mixture of incompatible variants. The lock
records each group's definition, effective Python requirement, and supported environment.

Resolution markers include an internal workspace-group selector alongside ordinary Python/platform
markers and dependency-group/extra conflict markers. The selector namespace is distinct from the
encodings used for actual extras and dependency groups. Successful resolutions are merged by package
identity; identical package records and dependency edges are shared rather than duplicated solely
because several groups reach them.

Selecting a group simplifies its selector to true and the other selectors to false, restricts the
graph to its environment, and retains packages reachable from its roots. The resulting ordinary lock
view is used by installation and export. Group definitions and their environments participate in
lock freshness checks.
