# uv Git checkout marker follows a repository-controlled symlink

Issue: astral-sh/uv#21857

Classification: bug

## Summary

The report demonstrates that a Git dependency can track `.ok` as a symlink to an existing writable
file outside its checkout. A successful `uv pip install` performs the checkout and then writes uv's
readiness marker through that symlink, truncating the external target. The reporter reproduced this
on uv 0.12.13 and on the cited current-main revision for uv 0.12.17; native `git clone` followed by
`git reset --hard` did not alter the target.

The repository source supports the reported mechanism. `uv-git` defines
`CHECKOUT_READY_LOCK` as `.ok` inside the checkout, treats its existence as evidence that a checkout
is fresh, removes it before reset, and calls `paths::create` on the same worktree path after
`git reset --hard` and submodule processing succeed. Because Git controls the worktree entry after
the reset, a tracked symlink can redirect that final marker creation outside the checkout.

No existing issue or pull request was found for this exact Git checkout-marker collision. The
closest precedent is astral-sh/uv#19542, fixed by astral-sh/uv#19543, where `uv cache prune`
followed a symlink and deleted its target outside the cache. That fix was confined to pruning in
`uv-cache` and does not protect marker creation in `uv-git`.

## Draft response

Thanks for the report. The current checkout code does reserve `.ok` inside the Git worktree and
recreates it after `git reset --hard`, so a repository-tracked symlink at that path can redirect the
marker write outside the checkout. That is incorrect behavior, and we'll keep astral-sh/uv#21857
as a separate bug.

The closest prior issue is astral-sh/uv#19542, fixed by astral-sh/uv#19543, but that change only
prevents `uv cache prune` from following cache symlinks; it does not cover marker creation in
`uv-git`. The concrete next step here is a regression test using a Git dependency that tracks `.ok`
as a symlink, followed by changing the readiness-marker handling so it cannot follow or rely on a
repository-controlled entry.

## Classification

This is a bug. The reported successful install has an unintended side effect outside the checkout,
and the relevant source path confirms that uv writes a reserved marker name after the repository
has populated that same worktree path. The incorrect external mutation is established independently
of whether the eventual remediation rejects a tracked `.ok`, creates the marker without following
symlinks, or moves checkout state outside the repository-controlled tree.

This is not a duplicate or a regression of astral-sh/uv#19542 or astral-sh/uv#19543. Those items
cover a different command and implementation path: canonicalization before deletion during cache
pruning. Their merged fix changed `uv-cache`; the current report reaches `paths::create` from the
Git checkout reset path in `uv-git`.

## Search and supporting evidence

Literal searches covered `.ok`, checkout marker, checkout readiness, repository-controlled and
tracked symlinks, writes outside the checkout, and truncation. Conceptual searches covered following
symlinks, unsafe paths, arbitrary or external file mutation, cache escape, malicious Git
dependencies, and path traversal. Open and closed issues and open, closed, and merged pull requests
were searched. Fix-oriented review followed the closest issue through its comments and merged fix.

astral-sh/uv#8731 was also inspected because it involved a package causing writes outside the
intended destination. It is not closely related enough to include below: its confirmed mechanism was
an unsanitized ZIP member path during wheel extraction, not a Git checkout marker or a symlink.

## Related

- astral-sh/uv#19542 — Closed issue and closest conceptual precedent. `uv cache prune` canonicalized
  a symlink in a cache bucket and could delete the external target. Both reports concern uv following
  a symlink and mutating data outside its intended tree, but the command, entry ownership, operation,
  and implementation path differ.
- astral-sh/uv#19543 — Merged fix for astral-sh/uv#19542. It stopped cache pruning from passing
  canonicalized symlink targets to recursive deletion and added a regression test. It establishes a
  relevant safe-handling pattern, but its `uv-cache` changes do not cover the `uv-git` `.ok` marker.
