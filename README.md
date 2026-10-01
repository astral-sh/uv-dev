# Support per-project environment constraints for workspace dependency resolution

Issue: astral-sh/uv#22115

Classification: duplicate

## Summary

The report asks for a PEP 508 environment constraint on each workspace member. In the example,
Windows-only and Linux-only members pin incompatible NumPy versions. Although those members are
never intended to be installed together, uv currently treats both as workspace roots during the
universal solve. The proposed setting would make resolution conditional on each member's supported
environment, reject an explicit request for an incompatible member, and make `uv sync
--all-packages` select only members compatible with the active environment.

astral-sh/uv#5594 is the canonical open design for this underlying capability. It describes
forking workspace roots by environment, omitting incompatible members, and rejecting explicit
selection of an incompatible member. Its concrete marker dimension is Python version; this report
generalizes the same model to operating-system and other PEP 508 markers.

## Draft response

Thanks for the detailed proposal. This is the same underlying workspace-resolution model tracked in
astral-sh/uv#5594: fork the workspace roots by environment, omit incompatible members for the active
environment, and reject explicitly selecting an incompatible member. That discussion is currently
framed around Python versions; your example shows why the design should extend to general PEP 508
markers such as operating-system constraints.

Package-level conflicts from astral-sh/uv#14906 can make mutually incompatible members jointly
lockable today, but they do not declare platform eligibility or make `uv sync --all-packages`
select only compatible members. Let's centralize the design discussion in astral-sh/uv#5594.

## Classification

Duplicate of astral-sh/uv#5594. Both reports require the resolver to condition workspace roots on
the environment, sync only compatible members, and fail when an incompatible member is selected
explicitly. The new issue contributes an operating-system example and proposes arbitrary PEP 508
markers, but this is a generalization of the open design rather than a separate correctness
regression.

The current behavior is an explicit workspace limitation: repository documentation says workspace
members must have compatible requirements and that the workspace uses one combined resolution.
Maintainers also classified the later Python-marker reproduction in astral-sh/uv#19576 as an
enhancement. The precedence rule therefore makes `duplicate` more appropriate than `enhancement`
for this new report.

## Related

- astral-sh/uv#5594 — Open RFC and canonical match. It proposes environment-dependent workspace
  roots, omission of incompatible members from the active environment, and an error when an
  incompatible member is selected. The difference is that it uses Python versions rather than
  arbitrary PEP 508 markers as the concrete environment dimension.
- astral-sh/uv#19576 — Open enhancement with a current reproduction of the same limitation. uv
  intersects every workspace member's Python range even when a dependency marker makes a member
  relevant only in another environment.
- astral-sh/uv#14012 — Closed question covering the `--all-packages` portion on an
  architecture-specific host. The maintainer-provided workaround is a root dependency with a
  platform marker; it does not provide per-member eligibility or strict selection errors.
- astral-sh/uv#13735 — Merged pull request implementing the analogous capability for dependency
  groups. One group-level `requires-python` replaces markers on every dependency, affects
  resolution, and errors when the group is selected with an incompatible interpreter. It does not
  cover workspace members or non-Python markers.
- astral-sh/uv#14906 — Merged pull request adding package-level workspace conflicts. It can make
  mutually incompatible members share one lock, but it does not bind members to PEP 508
  environments or filter `--all-packages` for the current platform.

## Search and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal terms
included `supported-environments`, workspace members, platform-specific packages, `--package`, and
`--all-packages`. Conceptual searches used the repository's vocabulary around limited and required
environments, diverging `requires-python`, conditional members, incompatible requirements,
package-level conflicts, universal resolution, and per-member lockfiles.

The reference chain from astral-sh/uv#5594 was inspected through astral-sh/uv#5398,
astral-sh/uv#5578, astral-sh/uv#5583, and astral-sh/uv#5644. astral-sh/uv#5398 explicitly directs
the workspace limitation to astral-sh/uv#5594. astral-sh/uv#5644 describes its intersection-based
behavior as an interim choice and points to astral-sh/uv#5594 for the longer-term strategy.

Plausible but distinct candidates were also checked. astral-sh/uv#11463 concerns lock-wide platform
filtering and wheel metadata fetches, not member eligibility. astral-sh/uv#9150 requests separate
lockfiles and virtual environments rather than one environment-dependent universal lock.
astral-sh/uv#21670 concerns concurrent hardware-specific virtual environments, not conditional
workspace roots.
