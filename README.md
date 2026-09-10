# uvx should be able to load packages from different internal indexes

Issue: astral-sh/uv#21358

Classification: duplicate

## Summary

The report describes an organization with eleven GitLab project package indexes configured for
`uvx` and `uv tool install`. GitLab forwards package misses to PyPI, so uv's default `first-index`
strategy can accept a same-named public package from whichever private index is listed first rather
than continue to the private index that actually hosts the organization's package. The same problem
applies to internal transitive dependencies. Resolution succeeds without warning and can therefore
run an unrelated public command.

The requested capability is a package-to-index mapping outside project or PEP 723 metadata. The
report proposes three interfaces: sources in `uv.toml`, a per-package source CLI option, or reading
the target project's uv configuration for local-path tool installs.

In a follow-up, the reporter withdrew the claim that the immediate incident is an uv defect. They
now attribute it to GitLab's package-forwarding behavior: because each configured GitLab index can
proxy a miss to PyPI, the first index can report a public package as present before uv reaches the
internal index that owns the organization's package. Their intended operational resolution is to
disable GitLab package forwarding and avoid internal names that collide with public packages. This
is a reporter conclusion and workaround; there is no maintainer response in the discussion yet.

astral-sh/uv#8758 is the canonical match. It already tracks the requirement to pin an internal
package to an index for tool installs without public fallback, including the dependency-confusion
risk. The new GitLab forwarding topology is a more specific reproduction of that same missing
capability. astral-sh/uv#6772 and astral-sh/uv#15529 cover two of the proposed interfaces, while
merged astral-sh/uv#17455 provides only a partial CLI improvement.

## Classification

This is a duplicate of astral-sh/uv#8758. Both reports require a durable association between an
internal package name and its private index in the tool-install context so uv cannot select a
same-named public package. astral-sh/uv#8758's maintainer discussion explicitly says sources are not
respected for tool installs and leaves open how index pinning for tools should be supported.

The behavior is an intentional current limitation rather than a regression: repository
documentation says `explicit = true` indexes are usable only for packages pinned through
`tool.uv.sources`, and named source indexes must be declared in a project's `pyproject.toml`.
Repository code rejects `sources` in `uv.toml`. The documentation also confirms that the default
`first-index` strategy stops at the first index on which a package name is available. The reporter's
proxy-forwarding configuration makes that intentional strategy insufficient to identify which
private index owns each package, but the resulting tool pinning request is already tracked.

The follow-up does not change the duplicate classification. It retracts the immediate bug claim and
reduces the need for uv-side action in this deployment, but the package-to-index capability requested
in the issue body remains the same capability already tracked by astral-sh/uv#8758.

## Current disposition and workaround

The reporter's current position is that GitLab's forwarding configuration, combined with public
name collisions, causes the observed selection. They plan to address the deployment rather than
seek a new uv resolution behavior:

1. Disable package forwarding from the self-hosted GitLab registries to PyPI.
2. Rename or otherwise avoid internal packages whose names collide with packages on PyPI.

Disabling forwarding should allow a registry that does not host a package to return a miss, after
which uv can continue to the later configured index. Avoiding collisions removes the public package
that otherwise satisfies the forwarded request. These steps are reported plans, not independently
verified reproduction results.

## Related

- astral-sh/uv#8758 — **Pinning package to index without fallback** (open issue). This is the
  canonical match: it requests package-to-index pinning that cannot fall back to a same-named public
  package, and maintainer comments explicitly identify tool installs as the unresolved context.
- astral-sh/uv#6772 — **`[tool.uv.sources]` can't be used as `[sources]` in `uv.toml`** (open issue).
  This directly tracks the report's first proposed solution, but its discussion is broader and often
  concerns project-local overrides and lockfile validity rather than global tool resolution.
- astral-sh/uv#15529 — **`uv tool install /path/to/dir` should read `pyproject.toml` configuration in
  `/path/to/dir`** (open issue). This directly tracks the third proposed solution for local-path tool
  installs; it does not cover registry-installed tools or CLI source mappings.
- astral-sh/uv#17455 — **Support referencing indexes by name via `--index` and `--default-index`**
  (merged pull request). This recently added the ability to select configured indexes by CLI name
  under the `index-by-name` preview feature, but deliberately treats the selected index as
  non-explicit for the invocation and does not bind individual packages to different indexes.

## Search and supporting evidence

Literal searches covered `sources uv.toml`, `tool.uv.sources uv.toml`, `uvx private index`, `uvx
multiple indexes`, `uv tool install private index`, `tool install sources`, and package/index
pinning without fallback. Conceptual searches covered first-index behavior, dependency confusion,
private-index proxy or PyPI forwarding, explicit indexes, package-name collisions, and transitive
private dependencies. Fix-oriented searches covered open, closed, and merged pull requests for
sources in `uv.toml`, uvx/tool index handling, CLI source selection, reading tool-target project
configuration, and first-index behavior. Candidate discussions and cross-reference chains were
inspected, including the issues suggested by the reporter.

The strongest adjacent candidates were distinguished as follows:

- astral-sh/uv#9440 also asks for package-specific index pinning in user-level `uv.toml`, but
  astral-sh/uv#8758 is closer because its maintainer discussion explicitly reaches the tool-install
  design and no-public-fallback requirement.
- astral-sh/uv#8253 concerns sources for undeclared transitive dependencies inside a project. It is
  related to the report's transitive symptom, but it assumes project metadata exists and therefore
  does not track the missing global tool source mapping.
- astral-sh/uv#18053 was inspected and ruled out as a close match: it concerns a project source in
  `pyproject.toml` referencing an index declared separately in `uv.toml`, not `uvx` or tool-level
  package routing.
- astral-sh/uv#16021 confirms that tool installation from package artifacts does not generally read
  the package's non-standard uv configuration, but it was resolved as a support question and its
  concrete failure centered on authentication. astral-sh/uv#15529 is the active, narrower tracker
  for changing local-path behavior.

No open or closed pull request was found that implements per-package source routing for `uvx` or
`uv tool install`. The closest merged change is astral-sh/uv#17455, which does not solve the reported
multi-index ownership problem.
