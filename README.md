# Behaviour of "uv export" in workspace

Issue: astral-sh/uv#20912
Classification: bug

## Summary

The closest historical discussions establish that `--package` must restrict export to the selected workspace package's dependency tree. No existing report tracks the claimed uv 0.12.1 regression.

## Classification

Repository-backed maintainer guidance in astral-sh/uv#9278 establishes that `uv export --package` should traverse from only the selected package, and astral-sh/uv#16503 plus astral-sh/uv#16603 preserve that package-subset model. The reported change from package-specific output in uv 0.11 to the entire workspace graph in uv 0.12.1 therefore describes incorrect behavior. No existing open issue or pull request was found tracking this regression, so it is not a duplicate. The precise cause remains unconfirmed.

## Related

- https://github.com/astral-sh/uv/issues/9278 (closed issue): `uv export --package` from workspace root doesn't export root's dependencies
  The maintainer explicitly states that `uv export --package package1` should export the dependency tree rooted at that package, excluding unrelated workspace-root dependencies. This directly establishes the expected behavior contradicted by the reported uv 0.12.1 output.
- https://github.com/astral-sh/uv/issues/16503 (closed issue): uv export in a monorepo with workspaces: allow exporting dependencies for a subset of packages
  This is the canonical discussion of selecting workspace-package subsets for export. It confirms that exporting one package's dependencies already worked and requested extending selection to multiple packages.
- https://github.com/astral-sh/uv/pull/16603 (merged pull request): Accept multiple packages in `uv export`
  This implemented workspace-package subset selection and added integration coverage. It reinforces that `--package` is intended to restrict the exported dependency graph, though there is no source-backed evidence yet that this change caused the uv 0.12.1 regression.
