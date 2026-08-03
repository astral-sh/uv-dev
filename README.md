# workspace groups no longer additive for members

Issue: astral-sh/uv#20917
Classification: duplicate

## Summary

The same additive root-plus-member group behavior is already requested in astral-sh/uv#9863 and astral-sh/uv#11537. astral-sh/uv#20840 directly explains the v0.12.1 change to member-over-root precedence.

## Classification

astral-sh/uv#9863 and especially astral-sh/uv#11537 already track combining root and selected-member dependency groups, including optional additions under the same group name. The report's v0.12.1 timing is source-confirmed by astral-sh/uv#20840, which deliberately introduced member precedence; there is no evidence that a previously fixed bug regressed, so the existing open requests take duplicate precedence.

## Related

- https://github.com/astral-sh/uv/issues/9863 (open issue): uv workspace doesn't install development dependencies for packages
  Tracks the same requested union of root and selected-member development dependencies without installing every workspace package.
- https://github.com/astral-sh/uv/issues/11537 (open issue): Better sync API for groups with workspaces
  Requests syncing a root group with a selected member and explicitly anticipates additional dependencies in the member's same-named group, closely matching this report.
- https://github.com/astral-sh/uv/pull/20840 (merged pull request): Allow commands run in workspace members to use root dependency groups
  Direct release-history evidence: astral-sh/uv#20840 shipped in v0.12.1 and explicitly made member definitions override same-named root groups. Its integration tests confirm that uv sync selects only the member's overlapping group.
- https://github.com/astral-sh/uv/issues/13540 (closed issue): Allow using workspace root dependency groups from member
  The feature request implemented by astral-sh/uv#20840 establishes why root groups became available to selected members in v0.12.1, although it focused primarily on root-only groups rather than additive overlapping definitions.
