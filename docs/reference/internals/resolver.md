# Resolver internals

!!! tip

    This document describes how uv's resolver works. The
    [resolution concept documentation](../../concepts/resolution.md) describes how to use uv.

## Resolver

Resolution finds package versions that satisfy a set of requirements. It is equivalent to the
[SAT problem](https://en.wikipedia.org/wiki/Boolean_satisfiability_problem), so it is NP-complete.
In the worst case, a resolver must try every combination of package versions. However, practical
resolution differs from this theoretical model:

- Loading package and version metadata is the slowest part, even when metadata is cached.
- Some valid solutions are better than others. uv generally prefers the latest package versions.
- Package dependencies have useful structure. Version ranges are usually continuous, and nearby
  releases often have similar requirements.
- Most resolutions do not require backtracking. Previous version preferences can further reduce the
  work.
- Failures need clear error messages that identify conflicting packages and explain the cause.
- The order in which the resolver selects packages strongly affects performance and results.

uv uses [pubgrub-rs](https://github.com/pubgrub-rs/pubgrub), the Rust implementation of
[PubGrub](https://nex3.medium.com/pubgrub-2fb6470504f), an incremental version solver. uv uses the
following process:

- A partial solution records selected and undecided package versions. Initially, only the virtual
  root package is selected.
- The resolver selects the highest-priority undecided package. URL dependencies have the highest
  priority, followed by exact specifiers such as `==`, then less strict specifiers. Within each
  category, the resolver uses the order in which packages first appeared. This keeps the result
  deterministic.
- The resolver selects a compatible package version that is not already marked incompatible. It
  prefers versions from a lockfile, such as `uv.lock` or `-o requirements.txt`, and versions already
  installed in the environment. Unless another
  [resolution strategy](../../concepts/resolution.md#resolution-strategy) applies, it checks
  versions from highest to lowest.
- The resolver adds the selected version's requirements to the undecided packages. uv fetches their
  metadata in the background.
- The process continues until the resolver detects a conflict. For example, `a 2 -> c 1` and
  `b 2 -> c 2` require incompatible versions of `c`. PubGrub records `{a 2, b 2}` as an
  incompatibility, restores the partial solution to `a 2`, and selects another version of `b`.

Resolution succeeds when the resolver selects compatible versions for all packages. It fails when an
incompatibility includes the virtual root package. This means no combination of direct and indirect
dependencies can satisfy the requested versions. PubGrub uses recorded incompatibilities to identify
the packages involved in its error message.

!!! tip

    [Internals of the PubGrub algorithm](https://pubgrub-rs-guide.pages.dev/internals/intro)
    describes the algorithm in more detail.

uv also changes the order of two packages when they conflict repeatedly.

## Forking

Historically, Python resolvers did not support backtracking. Even with backtracking, resolution
often covered only one architecture, operating system, Python version, and Python implementation.
Some packages require different versions in different environments:

```
numpy>=2,<3 ; python_version >= "3.11"
numpy>=1.16,<2 ; python_version < "3.11"
```

Because Python allows only one installed version of each package, a simple resolver would reject
these requirements. Inspired by [Poetry](https://github.com/python-poetry/poetry), uv splits the
resolution when requirements for the same package have different markers.

In this example, the partial solution splits into one resolution for `python_version >= "3.11"` and
another for `python_version < "3.11"`.

If markers overlap or do not cover all environments, the resolver creates additional forks. For
example:

```
flask > 1 ; sys_platform == 'darwin'
flask > 2 ; sys_platform == 'win32'
flask
```

This creates forks for `sys_platform == 'darwin'`, `sys_platform == 'win32'`, and
`sys_platform != 'darwin' and sys_platform != 'win32'`.

Forks can be nested and depend on earlier forks. uv merges forks with identical packages to limit
their number.

!!! tip

    The logs from `uv lock -v` show forks through `Splitting resolution on ...`,
    `Solving split ... (requires-python: ...)`, and `Split ... resolution took ...` messages.

Split points depend on the order in which the resolver finds packages. Lockfile preferences can
change that order and produce different forks during the next resolution. To keep resolutions
stable, uv records `resolution-markers` for each fork and each package that differs between forks.
Later resolutions reuse the saved forks. Changed requirements can add new forks.

## Wheel tags

uv resolves environment markers universally, but wheel tags remain platform-specific. A wheel tag
can identify the Python version, Python implementation, operating system, and architecture. For
example, `torch-2.4.0-cp312-cp312-manylinux2014_aarch64.whl` only supports CPython 3.12 on arm64
Linux with `glibc>=2.17`, as required by the `manylinux2014` policy. In contrast,
`tqdm-4.66.4-py3-none-any.whl` supports all Python 3 versions and interpreters on every operating
system and architecture.

Most projects provide a source distribution that uv can build when no compatible wheel exists.
However, some packages, such as `torch`, do not publish source distributions. Installation then
fails on any Python version, operating system, or architecture without a matching wheel.

## Marker and wheel tag filtering

Each fork has a known set of possible markers. Non-universal resolution knows their exact values.
Universal resolution knows at least the Python version constraint. For example,
`requires-python = ">=3.12"` excludes `importlib_metadata; python_version < "3.10"` because that
dependency cannot apply. The `tool.uv.environments` setting can exclude requirements for other
environments. Each fork can also exclude requirements that conflict with its own markers.

Some marker values imply the values of other markers. uv normalizes `python_version` and
`python_full_version`, along with known `platform_system` and `sys_platform` values, into shared
representations. This lets equivalent markers match.

A version with a local tag, such as `1.2.3+localtag`, may not provide wheels for every platform. If
the base version, such as `1.2.3`, supports a missing platform, uv can fork and select the
appropriate version for each platform. This helps packages such as torch that use local tags for
different hardware accelerators. Wheel tags and markers do not have a one-to-one correspondence, but
uv can map common Windows, Linux, and macOS platforms.

## Metadata consistency

Like Poetry, uv requires every wheel for a specific package version and index to declare the same
dependencies in `Requires-Dist`. This includes wheels built from source distributions. More
generally, uv expects each wheel to contain the same `METADATA` file in its `dist-info` directory.

For example, numpy 2.3.2 has 73 wheels. Consistent metadata lets uv fetch metadata once instead of
making 73 requests. It also avoids tracking both PEP 508 markers and wheel tags. These systems do
not map directly: a wheel tag can include a glibc version, but a PEP 508 marker cannot represent it.
Tracking both would add significant complexity without a well-defined correspondence.

PEP 508 markers already let one dependency declaration apply to multiple platforms, including
`project.[optional-]dependencies`. If existing markers cannot express a platform difference,
extending PEP 508 markers is preferable to using wheel tags as a separate dependency system.

A source distribution must also produce metadata that matches published wheels. If no wheels exist,
repeated builds must produce the same metadata. Without this guarantee, dependency locking is not
reliable. For example, a build of package `A` may first declare `B>=2,<3`, producing a lockfile with
`A==1` and `B==2`. If a later build declares `B>=3,<4` and `C>=1,<2`, the locked `B` version is
incompatible and the lockfile has no candidate for `C`.

Resolving dependencies again would bypass the lockfile and introduce unreviewed packages, including
`C` and `B==3`. This creates reproducibility and security risks. Failing during installation also
creates problems when the failure occurs during deployment. uv can already fail when a package has
no source distribution and its wheels do not support the current platform. Although
[required environments](../../concepts/resolution.md#required-environments) can reduce this risk,
the setting is not widely known. Source distributions should not create the same problem.

Older torch and TensorFlow versions had inconsistent metadata, but recent versions are consistent.
No major package is known to have inconsistent metadata. However, Python packaging standards do not
require consistency, and proposals to enforce it were rejected
(https://discuss.python.org/t/enforcing-consistent-metadata-for-packages/50008).

Some packages contain native code that links to native code in another package, such as torch. They
may build against multiple torch versions, but each build requires the same torch version at
runtime. This causes problems because major package managers, including pip and uv, cache source
distribution builds. uv supports separate builds for the installed dependency version through
[ `tool.uv.extra-build-dependencies`](../../concepts/projects/config.md#augmenting-build-dependencies)
with `match-runtime = true`. Users must configure this workaround for each affected package because
current standards do not let package authors declare this requirement directly.

## Requires-python

uv ensures that a resolution supports every Python version declared by the project. For example, a
project with `requires-python = ">=3.9"` cannot use a dependency that requires Python 3.10 or later.
Rejecting that dependency keeps the resolution installable on Python 3.9 and avoids packages that
require newer syntax or standard library features.

uv ignores upper bounds on `requires-python`, with special handling for packages that only provide
ABI-specific wheels. For example, it ignores `<4` in `requires-python = ">=3.8,<4"`. Issue
[#4022](https://github.com/astral-sh/uv/issues/4022) and this
[DPO thread](https://discuss.python.org/t/requires-python-upper-limits/12663) discuss the tradeoffs
and alternatives.

Most projects cannot determine whether they support a new Python version before its release. Upper
bounds would prevent users from upgrading to or testing these versions. Exceptions include packages
that depend on the unstable C ABI or CPython internals, such as its bytecode format.

Adding a `requires-python` upper bound does not prevent installation on newer Python versions when
older package releases lack that bound. A resolver can select an older release instead.

uv selects dependency versions that support the entire `requires-python` range of the project. For
example, a project that requires Python 3.12 or later cannot use a dependency that requires Python
3.13 or later. The result would not support Python 3.12.

Applying the same rule to upper bounds would reduce the compatible dependency versions whenever a
project raised its upper bound. Resolution could then fail if no dependency version supported the
full range. Raising a lower bound has the opposite effect: it increases the set of compatible
dependency versions.

Conda works differently because its solver also selects the Python version and can choose an older
one. Conda can also update package metadata after release. PyPI metadata cannot change after
publication.

Ignoring upper bounds causes problems for packages such as numpy that use the version-specific
CPython C API. Each numpy release supports four Python minor versions. For example, numpy 2.0.0
provides wheels for CPython 3.9 through 3.12 and requires Python 3.9 or later. numpy 2.1.0 provides
wheels for CPython 3.10 through 3.13 and requires Python 3.10 or later.

Without forking, a project with `requires-python = ">=3.9"` and `numpy>=2,<3` would select numpy
2.0.0. That lockfile would not install on Python 3.13 or later. To avoid this, uv forks when it
rejects a package version that requires a newer Python version. The `--fork-strategy` option
controls this behavior. In this example, uv creates separate resolutions for Python `>=3.9,<3.10`
and `>=3.10`:

```
numpy==2.0.0; python_version >= "3.9" and python_version < "3.10"
numpy==2.1.0; python_version >= "3.10"
```

uv does consider a project-level upper bound when removing unused wheels from the lockfile. For
example, `requires-python = "==3.13.*"` excludes `cp312` and `cp314` wheels. This happens after
resolution and does not affect package selection.

## URL dependencies

A dependency can come from a package registry or a URL. Registry dependencies use a package name and
an optional version specifier. URL dependencies include requirements in the form `{name} @ {url}`
and dependencies with a `git`, `url`, `path`, or `workspace` source.

A package URL fixes both the package source and its implied version. Two different URLs for the same
package produce a resolution error because each URL acts like an exact version pin. A
[flat index](../../concepts/indexes.md#flat-indexes) can provide multiple URLs instead.

uv requires URLs to appear directly in the project, a
[workspace member](../../concepts/projects/workspaces.md), in a
[constraint](../../concepts/resolution.md#dependency-constraints), or in an
[override](../../concepts/resolution.md#dependency-overrides). Another URL dependency can also
declare a URL. Before resolution, uv discovers all direct and transitive URL dependencies and fixes
their package sources and versions.

uv does not allow index packages to declare URL dependencies for two reasons. First, this
restriction improves security and predictability. Registry distributions cannot point to external
distributions, which makes accessed URLs easier to audit. For example, when a project uses one index
and no URL dependencies, uv only installs packages from that index.

Second, URL dependencies can introduce package versions that do not exist in the index. For example,
suppose a project depends on `foo`, `bar`, and `baz`. If `foo` requires `bar >= 2` but the index
only contains `bar` version 1, resolution should fail. However, a transitive `baz` dependency could
add `bar @ https://example.com/bar-2-py3-none-any.whl` and make the requirements resolve.

Allowing this would require uv to inspect every reachable package version before rejecting any
candidate. That breaks the incremental resolver assumption that the available versions of a package
do not change during resolution.

## Prioritization

Prioritization improves resolution speed and package selection.

Trying versions that the resolver later rejects requires extra metadata requests and additional
conflict tracking.

When multiple solutions satisfy the version constraints, uv prefers newer direct dependencies over
newer indirect dependencies. It also avoids very old package versions and selects packages that can
be installed on the target platform.

Internally, uv represents one package name with several virtual packages. These can represent active
extras, dependency groups, or markers. PubGrub selects a version for each virtual package, but uv
assigns priorities by package name.

The root package and URL requirements have the highest priority. Exact requirements with `==` come
next because their versions are known. Packages that conflict frequently follow, then all remaining
packages. Within each category, uv uses the order in which it first found each package. This creates
a breadth-first search that prioritizes direct and workspace dependencies over transitive
dependencies.

A common conflict occurs when package `A` has a higher priority than package `B`, but `B` only
supports older versions of `A`. After uv selects the latest `A` version, it rejects each `B` version
that conflicts with it. This can require many attempts, select an unsuitable old version, or fail
while building an old package.

After five such conflicts, uv gives both packages special priorities and selects `B` before `A`. It
then backtracks to the state before selecting `A` and continues with the new order. Issue
[#8157](https://github.com/astral-sh/uv/issues/8157) and pull request
[#9843](https://github.com/astral-sh/uv/pull/9843) describe real-world examples.
