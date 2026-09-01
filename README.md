# Allow for workspace members outside of workspace root

Issue: astral-sh/uv#21305

Classification: enhancement

## Summary

The reporter defines a workspace in `example/a/pyproject.toml` whose `members` contains the sibling
path `../b`. uv finds static metadata for both `a` and `b`, but while processing `b` it reports
`No workspace root found, using project root`; `b` then cannot satisfy its declaration
`a = { workspace = true }` because `a` is not a member of the implicit workspace rooted at `b`.

This is a request for a new workspace layout, not a regression in supported behavior. The workspace
implementation states that every member must be below the workspace root, and member-side workspace
discovery searches ancestor directories. A sibling root at `../a` therefore cannot be discovered
from `b` under the current model.

No existing issue or pull request tracks true workspace members outside their root. The closest
flat-layout discussion, astral-sh/uv#13589, recommends putting a project-less virtual workspace root
in the siblings' common parent. For projects that should remain separate workspaces,
astral-sh/uv#18348 and merged astral-sh/uv#18401 added path-valued workspace sources, but that feature
does not create shared membership or reuse the external workspace's lockfile.

The reporter has clarified that the real layout is a large monorepo with relevant projects scattered
across it. A single workspace file at their common ancestor is undesirable because multiple teams
may need independent workspaces rooted from that same directory. Moving the selected projects under
one directory is possible but would require reorganizing the monorepo.

A maintainer suggested allowing a root `uv.toml` to define multiple workspaces. The reporter said
that model would meet the use case. This is a design suggestion in the discussion, not an accepted
or implemented direction.

## Maintainer discussion

The supported common-parent virtual-root approach from astral-sh/uv#13589 does not address the
reported organizational requirement: multiple teams may want distinct selections of scattered
projects while sharing the same monorepo ancestor. A single `[tool.uv.workspace]` at that ancestor
can describe only one workspace.

Maintainer @zanieb floated a root `uv.toml` capable of defining multiple workspaces, and the reporter
confirmed that would work. The proposal would retain a discoverable common ancestor without forcing
all teams into one workspace, but its precise configuration and discovery semantics have not been
designed in the discussion.

Maintainer @zsol has now asked what specifically prevents moving the projects beneath a common
parent directory. The reporter has not yet answered that narrower question. The remaining useful
clarification is whether repository layout is constrained by ownership, build tooling, source
history, or another concrete requirement beyond convenience. If the projects are intended to remain
independently locked, path-valued workspace sources from astral-sh/uv#18401 remain a separate
alternative rather than a shared-workspace solution.

## Classification

`enhancement` is the best fit. The requested behavior conflicts with the current workspace model's
explicit structural invariant that members are below their root. The observed error follows from
the confirmed discovery mechanism: when uv processes `b`, workspace discovery walks upward from
`example/b`, so it cannot encounter the sibling `example/a` root and instead treats `b` as its own
implicit workspace. Supporting this layout would require new discovery and workspace-ownership
semantics (or another explicit way to associate an external member with its root).

The classification remains `enhancement`. The reporter has established why one common-parent
workspace is insufficient for the intended multi-team setup, and confirmed that a root configuration
capable of defining multiple workspaces would satisfy it. Maintainers have not selected that design,
and the practical constraint against regrouping the projects remains under discussion.

This is not a duplicate. astral-sh/uv#13589 covers the same broad flat-sibling topology but answers
it with a virtual root at the common parent. astral-sh/uv#18348 and astral-sh/uv#18401 cover
dependencies sourced from another workspace, deliberately without making those packages members of
one shared workspace. astral-sh/uv#13298 has the same final error but concerns descendants behind an
intervening project, where the intended workspace root is still an ancestor.

## Related

- astral-sh/uv#13589 — `using uv workspaces in flat layouts (e.g. monorepos)` (open issue). This is
  the closest layout discussion: sibling projects in a flat directory need to see one another. A
  maintainer recommends a project-less virtual workspace root in their common parent; unlike this
  request, it does not make one sibling the root of a workspace containing another sibling.
- astral-sh/uv#13298 — `Are doubly-nested packages supported in a workspace?` (open issue). It shows
  the same failure to discover the intended root and the same `workspace = true` not-a-member error.
  Its trigger is different: all members are below the root, but an intervening project stops
  discovery. That issue tracks nested descendants, not external members.
- astral-sh/uv#18348 — `Add workspace_dir to tool.uv.sources` (closed issue). This adjacent request
  sought dependency lookup in another workspace and led to a supported option for separate
  workspace trees; it did not request shared membership.
- astral-sh/uv#18401 — `Allow workspace to take a path in tool.uv.sources` (merged pull request).
  This implements path-valued workspace sources. Its description explicitly says uv does not reuse
  the referenced workspace's lockfile and that the feature is effectively sugar for path sources to
  individual members, distinguishing it from the capability requested here.

## Search evidence

Authenticated searches covered open and closed issues and open, closed, and merged pull requests.
Literal searches included `workspace members outside`, `outside workspace root`, `../` with
`workspace = true`, `No workspace root found`, and the exact not-a-workspace-member error.
Conceptual searches covered external and sibling members, flat monorepos, parent-directory members,
workspace-root discovery, environment or project-root overrides, and dependencies across different
workspaces. Fix-oriented searches examined relative-path and workspace-discovery changes and followed
issue-to-pull-request closure links.

astral-sh/uv#16285 and its merged fix astral-sh/uv#16296 were plausible exact-error matches but were
ruled out because they concern a leading `./` on members already below the root. astral-sh/uv#16640
also concerns root discovery, but only for nested descendants. astral-sh/uv#17156 confirms that uv
normally finds a package's workspace by traversing upward, while addressing conflicting sources in
nested workspaces rather than external members.
