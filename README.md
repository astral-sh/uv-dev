# uv Git checkout marker follows a repository-controlled symlink

Issue: astral-sh/uv#21857

Classification: bug

## Summary

The reported behavior is reproducible with the installed uv 0.12.13 on Linux. A minimal local Git
dependency tracked `.ok` as an absolute symlink to a 16-byte file outside the repository. A
successful `uv pip install` truncated that file to zero bytes, while a native `git clone` followed
by `git reset --hard` left it unchanged. The installed package remained usable after uv reported a
successful build and install.

The observed cached checkout retained `.ok` as a symlink to the external file. This is consistent
with the repository implementation: `uv-git` defines
`CHECKOUT_READY_LOCK` as `.ok` inside the checkout, treats its existence as evidence that a checkout
is fresh, removes it before reset, and calls `paths::create` on the same worktree path after
`git reset --hard` and submodule processing succeed. Because Git controls the worktree entry after
the reset, a tracked symlink can redirect that final marker creation outside the checkout.

No earlier issue or pull request was found for this exact Git checkout-marker collision. An
external contributor has since opened astral-sh/uv#21892 in response to this report. The closest
historical precedent is astral-sh/uv#19542, fixed by astral-sh/uv#19543, where `uv cache prune`
followed a symlink and deleted its target outside the cache. That fix was confined to pruning in
`uv-cache` and does not protect marker creation in `uv-git`.

## Reproduction

Outcome: reproducible.

Environment:

- uv 0.12.13 (`x86_64-unknown-linux-gnu`), the affected release named in the report
- Linux 6.17.0-1022-azure x86_64
- Git 2.55.0
- CPython 3.12.3 at `/usr/bin/python3`

The fixture was created entirely under `$RUNNER_TEMP/issue-21857-reproduction`. It contained a
minimal setuptools project committed to a local Git repository, plus a tracked `.ok` symlink whose
absolute target was `$RUNNER_TEMP/issue-21857-reproduction/victim.txt`. `git ls-tree HEAD .ok`
reported mode `120000`, confirming that the symlink was repository-controlled. The victim initially
contained `do-not-truncate` and was 16 bytes long.

The native-Git control succeeded without changing the victim:

```console
git clone -q "$REPRO/source" "$REPRO/native/checkout"
git -C "$REPRO/native/checkout" reset --hard -q "$REVISION"
# victim: 16 bytes; SHA-256 924bd5a93a257e1787fb6bfb2fac6e44b041e96db03119d633c90645ff6333a4
```

Using a fresh isolated cache and install target, the uv operation was:

```console
UV_CACHE_DIR="$REPRO/cache-success" UV_PYTHON_DOWNLOADS=never \
  uv pip install --python /usr/bin/python3 --target "$REPRO/install-success" \
  "uv-git-ok-repro @ git+file://$REPRO/source@$REVISION"
```

uv exited successfully, built and installed `uv-git-ok-repro==0.1.0`, and the installed module
imported successfully. Afterward, the victim was 0 bytes with the empty-file SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`. The checkout entry under
`cache-success/git-v0/checkouts/.../.ok` was still a symlink to the victim. This directly reproduces
the reported external truncation during a successful install and distinguishes it from native Git
checkout behavior.

No existing integration test covers a repository-controlled `.ok` symlink. The nearest coverage is
`crates/uv/tests/project/edit.rs::add_git_lfs`, which identifies the checkout `.ok` path, asserts
that the marker is absent after an incomplete Git LFS checkout, and verifies recovery after the
marker is removed. It does not create `.ok` as a tracked repository entry or test an external
symlink target.

## Proposed fix status

astral-sh/uv#21892 is an open, non-draft pull request created in response to this issue. At the time
of review it was marked mergeable, had no maintainer review, and had not been merged. Its proposed
changes are limited to `uv-git` plus test dependencies:

- use `symlink_metadata` in `is_fresh` so a symlink does not qualify as a valid checkout marker;
- remove any repository-created `.ok` entry after `git reset --hard` and before `paths::create`, so
  marker creation does not follow a tracked symlink; and
- add unit tests for replacing a marker symlink without modifying its target and for rejecting a
  symlink as a freshness marker.

The added tests directly exercise the removal-and-create sequence and the metadata predicate. They
do not run the complete Git dependency reproduction through `GitCheckout::reset`, so the existing
end-to-end reproduction remains useful when reviewing the proposed fix.

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

- astral-sh/uv#21892 — Open pull request created in response to astral-sh/uv#21857. It proposes
  rejecting symlinks as freshness markers and unlinking a repository-controlled `.ok` entry before
  creating uv's regular marker. It includes focused unit tests but has not yet received maintainer
  review or been merged.
- astral-sh/uv#19542 — Closed issue and closest conceptual precedent. `uv cache prune` canonicalized
  a symlink in a cache bucket and could delete the external target. Both reports concern uv following
  a symlink and mutating data outside its intended tree, but the command, entry ownership, operation,
  and implementation path differ.
- astral-sh/uv#19543 — Merged fix for astral-sh/uv#19542. It stopped cache pruning from passing
  canonicalized symlink targets to recursive deletion and added a regression test. It establishes a
  relevant safe-handling pattern, but its `uv-cache` changes do not cover the `uv-git` `.ok` marker.
