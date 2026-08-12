# uv-build: suppress "missing upper bound" warning via env var

Issue: astral-sh/uv#21074

Classification: enhancement

## Summary

The reporter pins the uv executable with mise and builds a project whose
`build-system.requires` contains unbounded `uv-build`. `uv build --wheel` emits the warning that
the requirement needs an upper bound. The requested capability is a truthy environment variable,
illustrated as `UV_NO_BUILD_UPPER_BOUND_WARNING`, that suppresses this warning for workflows where
uv is pinned elsewhere.

No duplicate was found. Repository documentation and implementation distinguish the uv executable
from the independently published `uv_build` PEP 517 backend. uv may use its bundled backend for a
compatible local build, but other frontends use the separately declared build requirement. A mise
pin therefore does not accompany every source-tree or source-distribution build and does not replace
the bound recorded in `pyproject.toml`.

## Draft response

Thanks. Pinning the uv executable with mise does not make the build-system requirement redundant.
uv can use its bundled `uv_build` backend as a fast path for compatible local builds, but `uv_build`
is also a standalone PEP 517 backend, and other build frontends will install the version declared in
`build-system.requires`. That declaration also travels with the source tree or source distribution,
while the mise pin generally does not, so an unbounded requirement can select a future breaking
`uv_build` release when the project is built elsewhere.

An environment-controlled opt-out would therefore be a new escape hatch rather than a correction to
the warning. We can use this issue to consider that enhancement. For now, the supported way to
silence the warning is to record a compatible upper bound in `build-system.requires`.

## Classification

This is an enhancement. The observed warning matches the documented and source-implemented policy:
the backend follows uv's versioning policy, and the upper bound protects builds of an immutable
source distribution from a future breaking backend release. Pinning the local uv frontend narrows
the reporter's workflow but does not constrain all consumers of the build metadata. The issue asks
for a new suppression setting and does not establish incorrect existing behavior. No existing issue
or pull request tracks the same suppression capability, so duplicate does not apply. The live issue
also currently has the repository's `enhancement` label.

## Related

- astral-sh/uv#3957 — “Add a uv build backend” (closed): the canonical backend discussion
  establishes that `uv_build` is separately distributable and usable without uv, while uv can
  integrate with it through fast paths. https://github.com/astral-sh/uv/issues/3957
- astral-sh/uv#15000 — “Better handling of multiple uv-build versions for repackagers / distros”
  (open): the closest ongoing discussion of the costs of upper bounds and cases where downstreams
  remove or bypass them. It concerns repackager compatibility, not warning suppression.
  https://github.com/astral-sh/uv/issues/15000
- astral-sh/uv#20860 — “uv build's fast path for uv_build ignores both the declared version
  specifier and --build-constraints/--require-hashes” (open): this documents the bundled-backend
  fast path and the effect of the running uv version on local builds. It tracks enforcement of
  constraints and hashes, not suppression of the missing-upper-bound warning.
  https://github.com/astral-sh/uv/issues/20860

## Supporting evidence

The current build-backend documentation says that `uv_build` is a separate package, that uv uses its
bundled copy only when the declared requirement is compatible, and that other frontends use the
standalone package. The metadata check in `crates/uv-build-backend/src/metadata.rs` explicitly warns
because an unbounded backend can break a source distribution after a future breaking release.

The strongest superficially similar results were inspected and ruled out. astral-sh/uv#20128
reported a different warning that was incorrect for an in-tree wrapper backend and was fixed by
astral-sh/uv#20153. astral-sh/uv#14724 and its merged fix astral-sh/uv#14731 only standardized the
format of generated upper bounds; they did not provide an opt-out.

## Search coverage

Literal issue and pull-request searches covered the full warning text, `build_system.requires`,
`uv_build`, `UV_NO_BUILD_UPPER_BOUND_WARNING`, mise, suppression, disabling, and pinning. Conceptual
searches covered bundled, direct, and fast-path backends; standalone PEP 517 backend distribution;
build-system constraints; warning configuration; and upper-bound policy. Fix-oriented searches and
inspection covered the original backend architecture, merged upper-bound and warning changes, and
the open fast-path hashes and constraints fix. Open and closed issues and open, closed, and merged
pull requests were included.
