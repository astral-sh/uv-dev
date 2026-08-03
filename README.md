# File collisions between two distributions are resolved silently, survive uninstall of both, and uv sync neither detects nor restores the missing files

Issue: astral-sh/uv#20907
Classification: duplicate

## Summary

The report is best centralized in astral-sh/uv#15357 and astral-sh/uv#15238. Existing work covers collision diagnostics, broken overlap-aware uninstall behavior, and sync treating surviving metadata as sufficient; the aws-cdk case supplies another concrete reproduction.

## Classification

The report combines already-tracked facets of the same overlapping-distribution problem: astral-sh/uv#15357 is the canonical open collision-warning tracker, while astral-sh/uv#15238 already tracks broken uninstallation and sync failing to restore files because dist-info remains. The aws-cdk reproduction and the observation that a shared file can survive uninstalling both distributions add useful triggering detail, but do not establish a separate underlying problem. No evidence indicates a regression of a completed fix: collision warnings remain preview-only, and repair of existing incomplete environments is still open.

## Related

- https://github.com/astral-sh/uv/issues/15357 (open issue): Stabilize conflicting modules warning
  Canonical tracker for overlapping wheel modules causing nondeterministic installations and broken uninstallations. It tracks stabilizing the existing preview warning and explicitly references astral-sh/uv#15238.
- https://github.com/astral-sh/uv/issues/15238 (open issue): `uv sync` removes necessary dependencies when two packages include the same module
  Tracks the same RECORD-overlap failure: uninstall removes shared package files while dist-info remains, and later sync considers the distribution installed and does not restore it. Maintainers explicitly proposed detecting incomplete installs or handling files claimed by another RECORD.
- https://github.com/astral-sh/uv/issues/16546 (open issue): report an error when installing a package that overrides another
  Matches the request to report or reject silent collisions. Maintainers confirm conflict detection currently exists only through the preview feature and is not enabled by default because legitimate packages also overlap.
- https://github.com/astral-sh/uv/pull/13437 (merged pull request): Warn when two packages write to the same module
  Implemented warnings for top-level module collisions installed in one operation and names the conflicting wheels. Its scope is narrower than the report: it does not generally detect arbitrary shared paths or repair incomplete environments.
- https://github.com/astral-sh/uv/issues/19412 (open issue): `uv sync` does not properly recover from an environment with faulty packages (opencv-contrib-python package problem)
  A second concrete reproduction where overlapping distributions leave files missing after uninstall and `uv sync` does not repair the environment, confirming the behavior beyond the reporter's aws-cdk packages.
