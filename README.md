# Add option to inherit cache keys from dependencies

Issue: astral-sh/uv#21296

Classification: enhancement

## Summary

The report describes a uv workspace in which the native package `bar` depends on the native package
`foo`. Both use scikit-build-core. Changes to C++ headers or sources in `foo` correctly invalidate
and rebuild `foo` because it defines matching `tool.uv.cache-keys`, but `bar` is not rebuilt merely
because its dependency was rebuilt. The reporter confirms that `bar` links against `foo`, so the
requested invalidation reflects native link-time coupling rather than only a Python dependency.
The reporter also states that `foo` is already declared as a build dependency of `bar`.
Copying `foo`'s cache-key globs into `bar` duplicates configuration and evaluates relative paths
from `bar`'s directory. The reporter proposed a dependency-aware cache-key entry such as
`{ inherit = "foo" }`; a maintainer subsequently reframed the desired outcome as declaring that
`bar` should rebuild whenever `foo` rebuilds, without necessarily inheriting `foo`'s raw key inputs.

No duplicate was found. Existing work introduced configurable per-project cache keys and
native-backend defaults, while adjacent workspace discussions cover same-project native rebuilds or
CI affected-package queries. None lets a dependent inherit another workspace member's evaluated
cache information.

## Maintainer follow-up

A maintainer suggested that workspace-level `cache-keys` may be a more general solution than
requiring every member to name the members whose keys it inherits. An initial example placed globs
such as `**/pyproject.toml`, `**/*.cpp`, and `**/*.h` at the workspace root, but a second maintainer
clarified that this would likely invalidate only the workspace root rather than every member.
Consequently, root-level globs alone would not provide member-level propagation.

The more concrete design suggested in the follow-up is an opt-in form such as
`cache-keys = { workspace = true }`, allowing a member to incorporate workspace cache information.
This remains a tentative proposal rather than a maintainer decision. The unresolved semantic issue
is which directory should be used as the root when resolving paths: the member, the workspace, or
some combination. Cache keys are currently per-build metadata, so loading workspace information
also crosses an existing metadata boundary.

The reporter clarified that there are two distinct capabilities. Workspace-level defaults could
deduplicate common key declarations, but they would not by themselves make a change to `foo`
invalidate its dependent `bar`. Dependency propagation requires `bar` to incorporate `foo`'s
evaluated cache information. For the proposed `{ inherit = "foo" }` form, the reporter's intended
path semantics are unambiguous: evaluate `foo`'s patterns relative to `foo`, then include that
evaluated result in `bar`'s cache key.

After confirming the native link, a maintainer distinguished that proposed implementation from the
actual invalidation requirement: `bar` may only need to declare that it rebuilds when `foo` does.
The tentative configuration suggested for that model is
`cache-depends = [{ package = "foo" }]`. This would avoid copying or re-rooting `foo`'s path
patterns by depending on `foo`'s rebuild state instead. The name and semantics are exploratory and
have not been accepted as a design.

The reporter then confirmed that `foo` is already declared as a build dependency of `bar`. A
maintainer suggested that uv may be able to derive the correct invalidation from that existing
relationship instead of requiring another cache-specific declaration and said they would
investigate. This is not yet a confirmed behavior change or implementation decision.

Invalidation should remain selective and follow dependency direction: a change to `foo` should
rebuild its dependent `bar`, while a change only to `bar` should not rebuild its dependency `foo` or
unrelated native packages. The maintainers do not currently expect plain root-level globs to cause
workspace-wide rebuilding, but those globs also do not appear to solve dependent invalidation.

For a package-scoped operation, the reporter expects `uv sync --package bar` after a `foo` source
change to rebuild `foo` first and then rebuild `bar` against the updated `foo`. This establishes both
the desired inclusion of an affected build dependency despite the package filter and the required
build order.

## Confirmed coupling and remaining reproduction detail

The reporter confirmed that `bar` links against `foo` and declares `foo` as a build dependency.
This explains why a rebuilt `foo` can require `bar` to be rebuilt and supports treating the
motivating case as native build-input invalidation. The discussion still does not specify the linked
artifact, the exact build-dependency declaration, how scikit-build-core locates it, or a minimal
workspace reproducer. Those details would help determine whether uv can infer the relationship from
existing metadata or needs an explicit cache-specific setting.

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
nor a duplicate. Maintainer follow-up identifies opt-in workspace cache information as a potentially
more general enhancement for shared defaults, while a separate `cache-depends`-style declaration is
being considered for dependency-directed rebuilding. A maintainer is also investigating whether the
existing build-dependency declaration should be sufficient. The exact behavior and any API remain
undecided.

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
