# `uv sync` validates a `build-system.requires` wheel against the wrong platform's hash from `uv.lock`

Issue: astral-sh/uv#21608

Classification: bug

## Summary

The reported hash mismatch is reproducible with a minimal local two-index analogue on Linux
x86_64. A project locks one artifact of `review-dep==1.0.0` for its supported environment, while
an editable dependency also requires `review-dep==1.0.0` in `[build-system].requires`. Isolated
build resolution selects a different, valid artifact from the default index. uv 0.12.11 and
0.12.13 reject that artifact using the hash of the project artifact; uv 0.12.10 accepts it and
completes the sync.

The version boundary matches astral-sh/uv#21223, released in uv 0.12.11. That change made project
sync derive isolated-build verification hashes from the full lockfile. The controlled reproduction
confirms that the expected and computed digests belong to the two deliberately distinct wheel
files, not to corrupted content.

## Classification

This is a regression bug. A valid build dependency artifact selected from the configured default
index is rejected solely because another source's artifact with the same package name and version
is present in `uv.lock`. The command succeeds on uv 0.12.10 and fails with the reported symptom
starting on uv 0.12.11.

## Reproduction

Outcome: **reproducible**.

The reproduction ran with uv 0.12.13 (`x86_64-unknown-linux-gnu`) on Ubuntu Linux x86_64 and
CPython 3.12.3. All project files, wheels, virtual environments, tool installations, and caches
were placed below `/tmp/uv-issue-21608.otP4oB`.

The fixture contains:

- `main`, which depends on editable `my-dep` and `review-dep==1.0.0`;
- `my-dep`, whose `[build-system].requires` contains `review-dep==1.0.0`;
- an explicit `trusted` local simple index and a default `other` local simple index, each
  serving a distinct valid `review_dep-1.0.0-py3-none-any.whl`;
- marker-selected project sources, with the trusted index selected on Linux and the other index
  selected outside Linux; and
- `[tool.uv] environments = ["sys_platform == 'linux'"]`, so the lock records only the supported
  Linux artifact.

The relevant commands were:

```console
$ uv lock --no-cache --python /usr/bin/python3.12
$ uv sync --frozen --no-cache --no-install-project --python /usr/bin/python3.12
```

`uv.lock` recorded only the trusted wheel with
`sha256:e8a3a55926fc1870b050a545982c21a65e85062c7917ad15c811466f6637e5b6`.
During the editable build, isolated build resolution selected the wheel from the default index.
Its independently calculated digest was
`sha256:1557868f97208f222bfea817018151e67e5c572e462a1170bd09f5700649b992`.
uv 0.12.13 failed with:

```text
Failed to install requirements from `build-system.requires`
Failed to download `review-dep==1.0.0`
Hash mismatch for `review-dep==1.0.0`

Expected:
  sha256:e8a3a55926fc1870b050a545982c21a65e85062c7917ad15c811466f6637e5b6

Computed:
  sha256:1557868f97208f222bfea817018151e67e5c572e462a1170bd09f5700649b992
```

The same lock and sync command also failed identically on uv 0.12.11. On uv 0.12.10, it built
`my-dep` and installed both packages successfully.

An initial variant without the restricted `environments` setting locked both source variants and
did not produce a hash mismatch because both digests were accepted. This identifies the minimal
configuration needed for the controlled reproduction; the original attachment itself was not
available in the triage event, so its exact environment restriction could not be compared.

Existing coverage in
`crates/uv/tests/lock/lock.rs::lock_sdist_url_locked_build_dependency_hash_mismatch` verifies
that a known locked build dependency is checked before execution and intentionally expects a
mismatch when an index serves changed bytes at the same URL. It does not cover a valid
platform/source-specific artifact with the same name and version as a different locked artifact.

## Related

- astral-sh/uv#21223 — **Verify locked source archives before building metadata**. This merged
  change shipped in uv 0.12.11 and introduced lock-derived verification for isolated build
  dependencies.
- astral-sh/uv#7059 — **Wrong hash is used for repeated `tensorflow-text`**. This closed issue
  reported a correct platform wheel being checked against another platform's hash in ordinary
  dependency resolution.
- astral-sh/uv#7060 — **Use distribution hash over registry hash**. This merged change fixed
  astral-sh/uv#7059 by preferring hashes for the selected distribution.
- astral-sh/uv#13112 — **SHA mismatch if wheel has no sha in the index, but sdist has**. This closed
  issue demonstrated a wheel being checked against the same package/version's sdist hash.
- astral-sh/uv#17157 — **Avoid enforcing incorrect hash in mixed-hash settings**. This merged
  change fixed astral-sh/uv#13112 by associating verification with the selected wheel or sdist.

## Implementation evidence

`crates/uv/src/commands/project/sync.rs` constructs the isolated-build hasher from the full lock
with `target.lock().hash_strategy(target.install_path())`. In
`crates/uv-resolver/src/lock/mod.rs`, registry hashes are collected under a registry
name/version identity. The observed 0.12.10-to-0.12.11 boundary and controlled two-wheel digests are
consistent with that lock-derived policy being applied to the alternate build-resolution artifact.

## Fix

Outcome: **fixed**.

Lock-derived hash verification now records the registry indexes associated with each locked
package/version. Distribution metadata exposes the resolved registry index, and hash validation
uses the locked hashes only when that distribution comes from the same logical index. This keeps
verification for a locked artifact served from its recorded index, including equivalent index URLs
with differing credential forms, while allowing an isolated build dependency to use a valid
same-name/version artifact from another configured index.

The parent integration regression in
`crates/uv/tests/lock/lock.rs::lock_editable_build_dependency_cross_index` now asserts a successful
`uv sync --frozen --no-cache --no-install-project` and installation of both the editable dependency
and its runtime dependency. The neighboring
`lock_sdist_url_locked_build_dependency_hash_mismatch` test still confirms that changed bytes from
the locked index are rejected. Focused validation also covered the lockfile hash-strategy unit test,
formatting, and Clippy for the affected library crates.

Pull request: https://github.com/astral-sh/uv-dev/pull/1442
