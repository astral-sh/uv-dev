# Add option to inherit cache keys from dependencies

Issue: astral-sh/uv#21296

Classification: enhancement

## Summary

The report describes a uv workspace in which the native package `bar` depends on the native package
`foo`. Both use scikit-build-core. Changes to C++ headers or sources in `foo` correctly invalidate
and rebuild `foo` because it defines matching `tool.uv.cache-keys`, but `bar` is not rebuilt merely
because its dependency was rebuilt. Copying `foo`'s cache-key globs into `bar` duplicates
configuration and evaluates relative paths from `bar`'s directory. The requested capability is a
dependency-aware cache-key entry such as `{ inherit = "foo" }` that incorporates `foo`'s evaluated
cache information into `bar`'s cache key.

No duplicate was found. Existing work introduced configurable per-project cache keys and
native-backend defaults, while adjacent workspace discussions cover same-project native rebuilds or
CI affected-package queries. None lets a dependent inherit another workspace member's evaluated
cache information.

## Maintainer follow-up

A maintainer suggested that workspace-level `cache-keys` may be a more general solution than
requiring every member to name the members whose keys it inherits. Under that model, a root
configuration such as globs for `**/pyproject.toml`, `**/*.cpp`, and `**/*.h` could globally
invalidate native workspace builds. The maintainer explicitly noted that this does not currently
work and presented it as a design direction, not a decision.

The main implementation concern raised is architectural: cache keys are currently per-build
metadata, so applying workspace-root configuration to member builds may cross the existing metadata
boundary. Further maintainer input is needed to choose between workspace-level invalidation,
explicit dependency inheritance, or another mechanism.

## Draft response

Thanks. `tool.uv.cache-keys` is currently evaluated independently for each local project, and there
is no key type that inherits another workspace member's evaluated cache information. The existing
native-build guidance in astral-sh/uv#12399 and astral-sh/uv#15809 handles changes to a project's
own inputs, while astral-sh/uv#16013 covers the related but separate problem of identifying affected
packages in CI.

This is therefore a new enhancement rather than existing supported behavior. For now, `bar` needs
to list the relevant `foo` paths in its own cache keys, relative to `bar`, or be added to
`reinstall-package` if always rebuilding it is acceptable. A dependency-aware form would need
design around direct versus transitive inheritance and workspace-only resolution, so we cannot
promise an implementation timeline.

## Classification

This is an enhancement. The reporter explicitly describes the current lack of a dependent rebuild
as expected and requests a new cache-key form. Repository documentation and source show that
`tool.uv.cache-keys` currently supports file, directory, Git, and environment inputs evaluated for
each project directory; there is no dependency-inheritance variant. Related reports establish the
per-project workaround but do not track this cross-package behavior, so the issue is neither a bug
nor a duplicate. A maintainer's follow-up confirms that workspace-level cache keys also do not work
at present and identifies them as a potentially more general enhancement, with the exact design
still undecided.

## Related

- astral-sh/uv#6255 — This closed enhancement is the original request for user-configurable project
  cache inputs. It established per-project `cache-keys`, but did not cover sharing or inheriting
  evaluated keys between workspace members.
- astral-sh/uv#7136 — This merged pull request implemented the current extensible cache-key
  abstraction and file/Git inputs. It is the implementation foundation for the requested new key
  kind, but it does not traverse package dependencies or inherit another project's cache
  information.
- astral-sh/uv#12399 — This closed question has the closest workspace/native-extension workflow.
  Maintainers recommended `cache-keys` to rebuild C extensions when their own inputs change; it
  does not address rebuilding a dependent because another workspace member changed.
- astral-sh/uv#15809 — This open question uses the same build backend and source-change symptom.
  The confirmed answer is to configure cache keys because uv cannot infer scikit-build-core inputs,
  but the discussion concerns one project's sources rather than inheritance across dependencies.
- astral-sh/uv#15705 — This merged pull request added C/C++ and Rust cache-key patterns to projects
  generated with scikit-build-core and maturin. It improves same-project defaults only and does not
  propagate invalidation to dependents.
- astral-sh/uv#16013 — This open question is conceptually adjacent because it asks for dependent
  workspace members to be treated as affected when a dependency changes. Its scope is querying
  affected release targets in CI, and a maintainer confirmed there is no uv-native facility; it
  does not track `uv sync` cache-key inheritance.

## Search evidence

The search covered literal queries for `cache-keys`, cache-key inheritance, relative globs, and
scikit-build-core, plus conceptual queries for workspace-dependent rebuilds, local-dependency
invalidation, affected dependents, transitive invalidation, and native-extension source changes.
Open and closed issues and open, closed, and merged pull requests were searched. Full discussions,
maintainer comments, closing references, and linked pull requests were inspected, including the
original cache-key implementation and later native-backend defaults.

astral-sh/uv#6356 was a plausible conceptual candidate but is a broad CI/Docker change-only-testing
discussion. astral-sh/uv#9191 concerns Docker metadata freshness, and astral-sh/uv#15224 concerns
forced non-editable reinstalls. None tracks inheritance of a dependency's evaluated cache inputs.
