# Index URLs with different credentials but the same registry endpoint are not deduplicated

Issue: astral-sh/uv#21281

Classification: bug

## Summary

In a workspace, the root project defines a private index whose URL contains user information.
Running `uv add <package> --index <index>` in a member can persist a second definition of the same
index in the member without that user information. If root and member requirements then select
their respective definitions for the same package, `uv lock` rejects them as conflicting indexes
even though their registry endpoint is otherwise identical.

Both reported behaviors were reproduced with a non-secret placeholder username and a public test
endpoint. No CodeArtifact access or real credentials were needed.

## Classification

This is a reproducible bug. `uv add` successfully creates a workspace configuration that a later
`uv lock` rejects solely because one index URL contains placeholder user information and the other
does not. The failure was also reproduced without network access using `--frozen` and an invalid
test endpoint, so it is not specific to CodeArtifact or authentication responses.

The confirmed root cause spans the producer and consumer paths. `uv add` only deduplicated indexes
against the target member document before writing the URL without credentials; it did not compare
the incoming index with workspace-root definitions. Separately, the resolver's per-package conflict
check compared `IndexMetadata` values using credential-sensitive `IndexUrl` equality instead of the
canonical index identity already used by index configuration lookup.

## Reproduction

Outcome: reproducible.

Environment used:

- Installed `uv 0.12.5 (x86_64-unknown-linux-gnu)`
- Linux x86_64
- CPython 3.12.3 at `/usr/bin/python3`
- All project files, cache, and Python-install state under `$RUNNER_TEMP`

The workspace root initially contained an empty project, `members = ["member"]`, and this index:

```toml
[[tool.uv.index]]
name = "private"
url = "https://<dummy-user>@pypi.org/simple"
explicit = true
```

The member was an empty project. From the member directory, this normal, non-frozen command
succeeded:

```console
uv add iniconfig==2.0.0 --index private=https://pypi.org/simple --no-sync
```

It resolved three workspace packages and added the dependency, source pin, and a second index to
the member:

```toml
[tool.uv.sources]
iniconfig = { index = "private" }

[[tool.uv.index]]
name = "private"
url = "https://pypi.org/simple"
```

Adding `iniconfig==2.0.0` to the root dependencies and selecting the root `private` index for it,
then running the following from the workspace root, failed with exit status 1:

```console
uv lock
```

The observed error was `Requirements contain conflicting indexes for package 'iniconfig' in all
marker environments`, listing the otherwise identical endpoint once with the placeholder username
and once without it. A second network-free variant used `uv add iniconfig --index private --frozen`
with an invalid endpoint; it also copied the root index into the member without user information,
and `uv lock --offline` produced the same conflict.

Before the parent regression was added, integration coverage in `crates/uv/tests/project/edit.rs`
was adjacent but did not cover the workspace failure. `add_index_credentials` verifies that
credentials supplied while adding are not persisted. `existing_index_credentials` verifies
same-document reuse when a configured index without credentials is matched against an authenticated
URL. The test
`crates/uv/tests/lock/lock.rs::lock_multiple_sources_index_overlapping_extras` covers the conflict
diagnostic for genuinely different endpoints. The parent regression
`add_index_credentials_workspace` now covers root/member index definitions that differ only by
credentials.

The reporter used macOS Darwin 25.5.0 arm64, uv 0.12.5, and Python 3.13.12. Since the same behavior
occurs on Linux with Python 3.12.3, neither the reported platform nor Python version appears
necessary for this reproduction.

## Fix

Outcome: fixed.

The index URL type now exposes the existing semantic identity check, which compares authentication
realm and canonical location without treating embedded credentials as part of the registry
identity. The `uv add` persistence path uses that check to reuse a workspace-root index when a
member receives an index with the same name, format, and location, so it no longer writes the
credential-free duplicate. The resolver uses the same check when validating per-package index
consistency, allowing existing root/member configurations that differ only by credentials while
continuing to reject genuinely different endpoints or formats.

The parent integration regression in `crates/uv/tests/project/edit.rs` was changed from snapshotting
the duplicate and conflict to asserting that the member contains only the source pin and that a
pre-existing duplicated configuration locks successfully. Focused debug-profile validation passed:

- `cargo test --package uv --test project edit::add_index_credentials_workspace -- --exact`
- `cargo test --package uv --test project edit::add_index_credentials -- --exact`
- `cargo test --package uv --test project edit::existing_index_credentials -- --exact`
- `cargo test --package uv --test lock lock::lock_multiple_sources_index_overlapping_extras -- --exact`
- `cargo +stable fmt --all`
- `git diff --check`

### Contributor follow-up

A commenter reports independently reproducing the issue locally and proposes the same two-part
approach used by the existing fix: avoid copying an inherited root index into a member, and compare
exact index endpoints without treating credentials as part of their identity while preserving path,
trailing-slash, and format distinctions. They asked for maintainer confirmation before submitting a
pull request with regression tests. The comment adds no configuration or reproduction details, so
the verified reproduction above remains the evidence for the fix; maintainers should coordinate
against astral-sh/uv-dev#842 to avoid duplicate implementation work.

## Related

- astral-sh/uv#17610 — Closest match for copying an index into a workspace member and shadowing a
  root definition; it does not cover credential-variant equality or the lock conflict.
- astral-sh/uv#17455 — Merged pull request adding support for resolving named `--index` and
  `--default-index` values from workspace configuration. Its implementation copies a resolved root
  index into a child package.
- astral-sh/uv#20678 — Tracks persisted member indexes not participating in candidate search, an
  adjacent workspace-index persistence problem.
- astral-sh/uv#20922 — Intended to address astral-sh/uv#20678 by searching member indexes; it does
  not address credential-sensitive equality.

Pull request: astral-sh/uv-dev#842
