# Workspace groups

!!! warning

    This is a design proposal. Workspace groups and the command-line options below are not
    implemented.

Workspaces share a lockfile so that related projects can agree on dependency versions. However, a
large workspace may contain alternative sets of applications that cannot all use the same versions
or support the same Python range. Resolving those sets independently permits unrelated dependencies
to drift; requiring explicit conflicts makes users describe consequences of their dependency graph.

A workspace group names a set of resolution roots. All groups are locked together, using shared
versions where possible and automatically forking where their requirements cannot be satisfied by a
single solution. Each group must be independently resolvable. A conflict within one group remains an
error.

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

The workspace's `members` and `exclude` fields continue to discover projects by path. A group's
`members` refer to the `project.name` values of already-discovered workspace members, not paths or
globs. Membership can overlap: `api` is a root of both groups above. A workspace root with a
`[project]` table is not implicitly added to every group.

Group names are unique within the workspace and occupy a separate namespace from
`[dependency-groups]`. Unknown members, duplicate group names, and multiple `default = true` groups
are configuration errors. At most one group is selected for an installation or export in the initial
design.

`requires-python` is optional. It adds a restriction to the Python compatibility of that group's
members; it cannot make an incompatible member support another Python version. An empty intersection
is an error that names the group and the incompatible declarations. A member outside the group
contributes requirements only when it is reached through a dependency. Conditional dependencies and
their Python restrictions remain conditional rather than restricting unrelated environments.

For example, if `worker` requires Python 3.12 and `next-worker` supports Python 3.12 through 3.14,
the `main` group can be limited to Python 3.12 without reducing `python-next` to that range. The
shared `api` member must still be compatible with each environment in which it is used.

## Command selection

`uv lock` resolves every declared group into the same `uv.lock`, regardless of the default group.
The proposed `--workspace-group <name>` option selects one group's roots and locked solution for
`uv sync`, `uv run`, and `uv export`:

```console
$ uv lock
$ uv sync --workspace-group main
$ uv export --workspace-group python-next --format requirements-txt
```

`default = true` supplies the group for an otherwise unqualified command. An explicit workspace
group takes precedence. `--group` continues to select dependency groups, not workspace groups. When
combined with an explicit workspace group, `--package` selects a package within that group's
reachable members and retains that group's resolution context.

Without a workspace-group selection or configured default, the command retains its normal project
and package targeting. If a grouped lockfile contains multiple incompatible solutions for that
target, the command must request an explicit group instead of choosing an arbitrary variant. An
ungrouped workspace keeps its existing locking, Python selection, and installation behavior.

Workspace groups do not introduce additional virtual environments, publish package metadata, or
change source and index precedence. Separate environment management and composing multiple named
groups in one command are outside the initial scope.

## Resolution model

The resolution domain gains a workspace-group dimension alongside its environment markers. A root
requirement carries the set of groups in which it is active. Transitive requirements retain that
provenance, including when the same package is reached from several groups.

For example, suppose `worker` requires `sqlalchemy<2`, `next-worker` requires `sqlalchemy>=2`, and
both groups permit the same version of `httpx`. The result should contain separate SQLAlchemy
choices for `main` and `python-next` and a shared `httpx` choice. The user does not declare a
`tool.uv.conflicts` entry for these workspace groups.

Ordinary resolver backtracking must run before an incompatibility is interpreted as a reason to
fork. A conflict involving requirements from different groups can split the affected group contexts
and retry resolution. A conflict whose requirements must coexist within one group is an ordinary
resolution failure. In particular, forking must not remove a required member or dependency to make
that group appear solvable.

The incompatibility may only become visible through transitive dependencies, so comparing the
groups' direct requirements is insufficient. Nor is a table of pairwise group conflicts sufficient:
three sets of requirements can be jointly incompatible while every pair is compatible. The resolver
needs enough provenance to attribute its incompatibility to the affected group contexts.

The lock's supported Python domain covers the union of the groups' supported environments, not the
intersection of every workspace member's declaration. Group-specific Python restrictions remain
attached to their contexts. Existing platform/Python marker forks and explicit dependency-group or
extra conflicts still apply inside those contexts.

The implementation should reuse [universal resolution](./resolver.md#forking) where appropriate, but
it must not eagerly expand every possible subset of workspace members or every pair of groups. The
expected input is a small set of named root sets, potentially containing thousands of members. Group
identifiers can be interned, and identical group sets and resolved subgraphs can be shared.

Sharing is a resolution objective, not a claim that the resolver computes a globally minimum number
of versions. A previous lockfile should retain stable group-specific choices. The precise policy for
preferring a shared version over an otherwise newer version still needs to be specified.

## Lockfile and implementation outline

The lockfile needs to record the group definitions, their supported Python environments, and the
group context selecting each divergent package or dependency edge. Shared package records should not
be duplicated solely because they are reachable from multiple groups. Selecting a group must produce
an unambiguous installable graph without another solve, including under `--frozen`.

The exact wire format is not settled. It must participate in lock freshness checks and prevent an
older reader from silently installing a mixture of incompatible group variants. Reusing the existing
conflict-marker encoding is only appropriate if it can express group identity without collisions
with extras or dependency groups.

The implementation divides into these parts:

1. Add the configuration model under `ToolUvWorkspace` and resolve group members through
   `Workspace::packages()`. Keep validated groups on the workspace types rather than passing raw
   `pyproject.toml` tables through commands.
2. Extend `LockTarget` to produce group-scoped roots and Python environments. Do not lower all
   discovered members into unconditional roots when resolving named groups.
3. Carry group provenance through requirement lowering and the resolver's incompatibility analysis.
   Extend the fork representation to split group contexts without making every member combination an
   independent solve.
4. Extend the resolver manifest, lockfile encoding, freshness checks, and installable graph
   traversal to retain and select group contexts.
5. Wire the command selector through Python discovery, locking, installation, and export. Keep
   `--package`, extras, and dependency-group filtering separate from workspace-group selection.

The first end-to-end case should use two overlapping groups with incompatible transitive versions of
one dependency, a compatible shared dependency, and different Python ranges. It should prove that
both groups export from one frozen lockfile, that a repeated lock is stable, and that a genuine
conflict inside either group remains an error. Additional cases should cover invalid configuration,
conditional member dependencies, higher-order conflicts, source/index differences, and unchanged
behavior in a workspace without groups.

## Remaining design questions

- How should the lock cover members omitted from every named group while retaining ordinary
  `--package` targeting? An implicit all-members resolution cannot be mandatory, since that would
  reintroduce the conflicts the named groups are intended to separate.
- What should an unqualified command do when its normal target is available in several groups with
  different solutions? Requiring an explicit group is the provisional behavior.
- How strongly should the solver prefer shared versions, and when should an upgrade reconsider
  previously recorded group forks?
- Which lockfile representation provides stable group identities, compact membership, and safe
  reader-version negotiation?
