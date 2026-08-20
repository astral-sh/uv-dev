# uv sync resolves relative paths in subdependencies different when run fresh or with uv.lock file

Issue: astral-sh/uv#21244

Classification: bug

## Summary

The report demonstrates an inconsistency in `uv sync` for a package selected from a Git
subdirectory whose metadata contains a relative dependency on a wheel elsewhere in that Git
repository. On a fresh sync, resolution writes a valid lockfile but installation rebases the wheel
path against the downstream project checkout, producing a nonexistent local absolute path and a
`failed to query metadata of file` error. Re-running with that lockfile succeeds because the locked
source retains the Git-relative `path=test-wheel/dist/...whl` value. The behavior reproduces with uv
0.12.0 and 0.12.5 on macOS and Linux.

No open issue or pull request already tracks this exact Git-subdirectory-plus-archive mismatch. The
closest implementation is merged astral-sh/uv#10072, which added support for pre-built archives in
Git repositories. Current integration coverage verifies the repository-root form of this scenario,
but not a parent dependency selected with `subdirectory`. Closed astral-sh/uv#9516 and
astral-sh/uv#19152 cover analogous source-identity failures for relative directory dependencies.

## Draft response

Thanks for the clear reproduction. This is a bug: astral-sh/uv#10072 added support for pre-built
archives inside Git repositories, and the fresh resolution and lockfile replay should preserve the
same repository-relative wheel path. The existing coverage exercises a Git dependency at the
repository root; your case adds a Git subdirectory, where the fresh sync incorrectly rebases the
wheel path against the downstream project even though the generated lockfile retains the correct
Git-relative path. The next step is to add coverage for a transitive wheel path from a Git
subdirectory and make the fresh installation request use the same path represented in `uv.lock`.

## Classification

This is a bug. Repository history and the current `sync_git_path_archive` integration test establish
that a relative archive inside a Git repository is an intended supported source. For the same
dependency graph, fresh resolution and lockfile replay must identify the same distribution; using a
downstream-project absolute path only during the fresh installation phase is incorrect. The report
also includes a concrete cross-platform reproduction and logs showing both forms of the requested
source.

This is not classified as a duplicate because the closest reports are closed and differ at the
important failure point. In astral-sh/uv#9516 and astral-sh/uv#19152, uv serialized relative
directory dependencies as machine-local checkout paths in the lockfile. In astral-sh/uv#21244, the
lockfile already has the correct Git-relative archive source, while the fresh sync's installation
request has the wrong path. Merged astral-sh/uv#10072 intended to support this source category, but
its existing test covers only a Git package at the repository root, leaving this subdirectory
variant uncovered.

## Related

- astral-sh/uv#10072 (merged pull request), “Add support for direct archive dependencies in Git” —
  The closest implementation evidence. It explicitly added support for pre-built wheels and source
  archives inside Git repositories. Its `sync_git_path_archive` coverage verifies a transitive wheel
  path through fresh lock and sync, but the parent Git package is at the repository root rather than
  selected via `subdirectory`.
- astral-sh/uv#9516 (closed issue), “Adding Git repo at subdirectory that points to source in same
  repo does not work” — The same repository topology and path-base concern: a package selected from
  a Git subdirectory depends on a sibling by relative path. It differs because the sibling is a
  directory and the bad checkout path was written into the lockfile. Merged astral-sh/uv#9594 fixed
  that case by expressing the dependency relative to the Git root.
- astral-sh/uv#19152 (closed issue), “`uv lock` with transitive poetry path depedencies results in
  machine-specific path in lockfile” — Another close source-identity failure for a transitive
  relative path discovered in Git-sourced metadata. Merged astral-sh/uv#19269 preserved the enclosing
  Git source for directory dependencies. It differs because its incorrect machine-specific path was
  serialized into `uv.lock`; the new issue's lockfile is correct and the live installation request
  is wrong.

## Search evidence

Literal searches covered the exact cache and metadata errors, `Path mismatch`, `relative path`,
`uv.lock`, first and second sync behavior, `GitPath`, and Git `path` fragments. Conceptual searches
covered Git subdirectories, transitive and path dependencies, local wheel dependencies, direct
archives in Git, cache portability, and fresh-versus-locked resolution. Fix-oriented searches
inspected merged path canonicalization, Git source preservation, and direct Git archive support.

astral-sh/uv#10012 was inspected as a plausible wheel-in-Git match but is not canonical: its package
dynamically generated absolute URLs containing the build working directory, while astral-sh/uv#21244
uses a stable relative wheel path and produces a correct portable lock entry. astral-sh/uv#18443 was
also ruled out because it requests relative local Git source URLs; it is not about resolving a
relative dependency inside a remote Git checkout.
