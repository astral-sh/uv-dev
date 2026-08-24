# Index URLs with different credentials but the same registry endpoint are not deduplicated

Issue: astral-sh/uv#21281

Classification: bug

## Summary

In a workspace, the root project defines a private CodeArtifact index using a URL that includes a
username. Running `uv add <package> --index <index>` for a workspace member succeeds, but writes a
second index definition into the member's `pyproject.toml` without the username. A later `uv lock`
then rejects requirements for the same package because the root and member definitions are treated
as conflicting indexes even though they identify the same registry endpoint.

The closest existing work concerns `uv add` copying or persisting indexes in workspace members:
astral-sh/uv#17610, astral-sh/uv#17455, and astral-sh/uv#20678. The open fix in
astral-sh/uv#20922 would make member indexes participate in candidate search, but it does not change
how credential variants are compared. No existing issue or pull request covers both the unwanted
member entry and the resulting credential-variant conflict.

## Draft response

Thanks for the report. This is a bug. The current add path can copy a workspace index into the
member configuration, and index URLs are persisted without embedded credentials. The resolver can
then treat that member entry and the credential-bearing root entry as conflicting indexes for the
same package.

This overlaps with the workspace index-copying behavior in astral-sh/uv#17610 and
astral-sh/uv#20678, but neither tracks the credential-variant conflict, so this should remain
separate. Could you provide a minimal root/member pair of `pyproject.toml` files and clarify whether
the `--index` argument is an index name or a URL? Please use only a dummy endpoint and placeholder
username, with no real credentials. That will let us cover both the unwanted member entry and the
subsequent `uv lock` failure in a regression test.

## Classification

This is a bug because repository source confirms two correctness gaps behind the reported behavior:

- The project-editing path compares an incoming index only with indexes in the target member
  document. It uses canonical URL comparison, which ignores credentials, but it never checks the
  equivalent root index before creating the member entry. When it persists the incoming URL, it
  deliberately removes credentials.
- The resolver's per-package conflict check compares index metadata directly. Since that metadata
  contains the original index URL, credential-bearing and credential-free variants can compare as
  different and produce the reported conflicting-index error. Elsewhere in the repository,
  canonical URL comparison explicitly strips credentials, establishing that credentials are not
  intended to distinguish the underlying canonical location.

The report is not a duplicate. astral-sh/uv#17610 covers a copied child index shadowing a root index,
and astral-sh/uv#20678 covers persisted child indexes not being searched. Neither tracks the direct
credential-sensitive comparison or this lock failure. The exact `--index` form is still useful for
selecting the right regression-test setup, but it is not required to establish the correctness
problem.

## Related

- astral-sh/uv#17610 — Open bug and the closest match for the workspace mutation. It shows that
  adding through a member copies an index into that member and can shadow the root definition. It
  does not cover credential-variant equality or the resulting lock conflict.
- astral-sh/uv#17455 — Merged pull request adding support for resolving named `--index` and
  `--default-index` values from workspace configuration. Its implementation copies a resolved root
  index into a child package, and its review explicitly discusses whether that copy is necessary and
  whether indexes should be deduplicated by URL.
- astral-sh/uv#20678 — Open bug involving the same `uv add --index` workspace-member persistence
  path. It tracks member indexes being written to files that are not consulted for candidate search,
  which is adjacent but distinct from treating root/member credential variants as conflicts.
- astral-sh/uv#20922 — Open pull request intended to close astral-sh/uv#20678 by searching workspace
  members' own indexes. It does not alter credential-sensitive index equality and therefore does not
  subsume astral-sh/uv#21281.

## Search evidence

Searches covered open and closed issues and open, closed, and merged pull requests using the exact
`Requirements contain conflicting indexes` fragment, `uv add --index` workspace persistence,
root/member index copying and shadowing, duplicate and same-endpoint terminology, username and
credential variants, CodeArtifact, URL canonicalization, lockfile validation, and historical fixes.
The strongest candidates and their comments, linked issues, and linked pull requests were inspected.

astral-sh/uv#18250 confirms in maintainer discussion that independently supplied configured and
command-line/environment indexes are not matched by URL, but its trigger is different.
astral-sh/uv#20635 and astral-sh/uv#20753 concern trailing-slash lock validation, while
astral-sh/uv#14511 documents why slash normalization can change semantics. astral-sh/uv#9942 shares
the error text but involves genuinely different package indexes under extras, and astral-sh/uv#8565
concerns credential-cache reuse. These were inspected and ruled out as canonical matches.
