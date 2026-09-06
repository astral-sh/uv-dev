# Resolution

Resolution converts a list of requirements into a compatible set of package versions. A resolver
searches for package versions that satisfy the requested requirements and each package's own
dependencies.

## Dependencies

Most projects and packages depend on other packages to work. A package declares each dependency as a
_requirement_, which includes a package name and acceptable versions. The current project's own
requirements are its _direct dependencies_. Dependencies of those packages are _indirect_ or
_transitive dependencies_.

!!! note

    See the [dependency specifiers
    page](https://packaging.python.org/en/latest/specifications/dependency-specifiers/)
    in the Python Packaging documentation for details about dependencies.

## Basic examples

Consider the following dependencies:

<!-- prettier-ignore -->
- The project depends on `foo` and `bar`.
- `foo` has one version, 1.0.0:
    - `foo 1.0.0` depends on `lib>=1.0.0`.
- `bar` has one version, 1.0.0:
    - `bar 1.0.0` depends on `lib>=2.0.0`.
- `lib` has two versions, 1.0.0 and 2.0.0. Both versions have no dependencies.

The resolver must select package versions that satisfy the project requirements. Because `foo` and
`bar` each have one version, it selects those versions. It must also select a version of the
transitive dependency `lib`. `foo 1.0.0` accepts both versions of `lib`, but `bar 1.0.0` requires
`lib>=2.0.0`. Therefore, the resolver selects `lib 2.0.0`.

Some requirements have more than one valid solution. Consider these dependencies:

<!-- prettier-ignore -->
- The project depends on `foo` and `bar`.
- `foo` has two versions, 1.0.0 and 2.0.0:
    - `foo 1.0.0` has no dependencies.
    - `foo 2.0.0` depends on `lib==2.0.0`.
- `bar` has two versions, 1.0.0 and 2.0.0:
    - `bar 1.0.0` has no dependencies.
    - `bar 2.0.0` depends on `lib==1.0.0`
- `lib` has two versions, 1.0.0 and 2.0.0. Both versions have no dependencies.

The resolver must select one version each of `foo` and `bar`. `foo 2.0.0` and `bar 2.0.0` require
different versions of `lib`, so they cannot coexist. The resolver can select `foo 1.0.0` with
`bar 2.0.0`, or `bar 1.0.0` with `foo 2.0.0`. Both results are valid. Different resolution
algorithms might select different results.

## Platform markers

Markers attach a condition to a requirement. For example, `bar ; python_version < "3.9"` installs
`bar` only on Python 3.8 and earlier.

Markers adjust package dependencies for the current environment or platform. For example, they can
select dependencies by operating system, CPU architecture, Python version, or Python implementation.

!!! note

    See the [environment
    markers](https://packaging.python.org/en/latest/specifications/dependency-specifiers/#environment-markers)
    section in the Python Packaging documentation for more details about markers.

Markers affect resolution because their values change the required dependencies. Most Python
resolvers evaluate markers for the _current_ platform, where they install the packages. However, a
lockfile created this way works only on that platform. Platform-independent, or "universal",
resolvers avoid this limitation.

uv supports both [platform-specific](#platform-specific-resolution) and
[universal](#universal-resolution) resolution.

## Platform-specific resolution

By default, uv's pip interface, such as [`uv pip compile`](../pip/compile.md), produces a
platform-specific resolution. This matches `pip-tools`. The uv project interface does not support
platform-specific resolution.

The `--python-platform` and `--python-version` options select another platform or Python version.
For example, `uv pip compile --python-platform linux --python-version 3.10 requirements.in` resolves
for Python 3.10 on Linux, even when run on macOS. During platform-specific resolution,
`--python-version` sets the exact Python version, not a lower bound.

!!! note

    Python environment markers describe more machine details than `--python-platform` can express.
    For example, the macOS `platform_version` marker includes the kernel build time. Package
    requirements can depend on these details. uv attempts to create a resolution for every machine
    on the target platform. This works for most packages, but complex requirements can reduce
    accuracy.

## Universal resolution

uv creates its lockfile (`uv.lock`) with universal resolution. The lockfile works across operating
systems, architectures, and supported Python versions. [Project](../concepts/projects/index.md)
commands such as `uv lock`, `uv sync`, and `uv add` create and update this file.

To use universal resolution with [`uv pip compile`](../pip/compile.md), pass `--universal`. The
resulting requirements file contains markers that identify the platforms for each dependency.

Universal resolution can include a package more than once if different platforms need different
versions or URLs. Markers select the appropriate version. Because it considers every marker,
universal resolution is often more constrained than platform-specific resolution.

Every required package must support the _entire_ `requires-python` range in `pyproject.toml`. For
example, a project with `requires-python = ">=3.8"` must have dependencies that support Python 3.8.
Resolution fails if every version of a dependency requires Python 3.9 or later. The project's
`requires-python` range must be a subset of each dependency's `requires-python` range.

By [default](#multi-version-resolution), uv selects the latest compatible dependency version for
each supported Python version. For example, a project can support Python 3.8 and later while its
latest dependency version requires Python 3.9. uv selects the latest dependency for Python 3.9 and
later, and an older compatible version for Python 3.8.

For dependency `requires-python` ranges, uv considers lower bounds and ignores upper bounds. For
example, it treats `>=3.8, <4` as `>=3.8`. Upper bounds often cause formally correct but impractical
resolutions. For example, a resolver can backtrack to the first published version without an upper
bound. See
[`Requires-Python` upper limits](https://discuss.python.org/t/requires-python-upper-limits/12663).

## Limited resolution environments

By default, the universal resolver considers all platforms and Python versions.

To limit resolution to specific platforms or Python versions, use the `environments` setting. It
accepts a list of
[PEP 508 environment markers](https://packaging.python.org/en/latest/specifications/dependency-specifiers/#environment-markers).
The `environments` setting _reduces_ the set of supported platforms.

For example, limit the lockfile to macOS and Linux:

```toml title="pyproject.toml"
[tool.uv]
environments = [
    "sys_platform == 'darwin'",
    "sys_platform == 'linux'",
]
```

To exclude alternative Python implementations:

```toml title="pyproject.toml"
[tool.uv]
environments = [
    "implementation_name == 'cpython'"
]
```

Entries in `environments` must be disjoint: they cannot overlap. For example,
`sys_platform == 'darwin'` and `sys_platform == 'linux'` are disjoint. However,
`sys_platform == 'darwin'` and `python_version >= '3.9'` overlap because both can be true.

## Required environments

Python packages can provide source distributions, built distributions (wheels), or both.
Installation requires a wheel. If no wheel supports the current platform or Python version, uv
builds a wheel from the source distribution and installs it.

Some packages, such as PyTorch, publish wheels without a source distribution. These packages work
_only_ on platforms with an available wheel. For example, a package that publishes only Linux wheels
cannot install on macOS or Windows.

Packages without source distributions complicate universal resolution because some platforms or
Python versions might not have a compatible wheel.

By default, each package without a source distribution must provide at least one wheel for the
target Python version. Use `required-environments` to require wheels for specific platforms. If no
matching wheel exists, resolution fails. This setting accepts a list of
[PEP 508 environment markers](https://packaging.python.org/en/latest/specifications/dependency-specifiers/#environment-markers).

The `environments` setting _limits_ the environments that uv considers. In contrast,
`required-environments` _expands_ the platforms that the resolution _must_ support.

For example, `environments = ["sys_platform == 'darwin'"]` limits resolution to macOS and excludes
Linux and Windows. In contrast, `required-environments = ["sys_platform == 'darwin'"]` requires a
macOS wheel for each package without a source distribution. Resolution fails if a required wheel is
missing.

Use `required-environments` to support older platforms that might require older package versions.
For example, require every wheel-only package to support Intel macOS:

```toml title="pyproject.toml"
[tool.uv]
required-environments = [
    "sys_platform == 'darwin' and platform_machine == 'x86_64'"
]
```

## Common marker values

The `environments` and `required-environments` settings accept
[PEP 508 environment markers](https://packaging.python.org/en/latest/specifications/dependency-specifiers/#environment-markers).
The Python runtime provides these marker values through functions such as
[`sys.platform`](https://docs.python.org/3/library/sys.html#sys.platform),
[`platform.machine()`](https://docs.python.org/3/library/platform.html#platform.machine),
[`platform.system()`](https://docs.python.org/3/library/platform.html#platform.system), and
[`os.name`](https://docs.python.org/3/library/os.html#os.name).

Common marker values include:

| Marker                      | Linux       | macOS      | Windows     |
| --------------------------- | ----------- | ---------- | ----------- |
| `sys_platform`              | `'linux'`   | `'darwin'` | `'win32'`   |
| `platform_system`           | `'Linux'`   | `'Darwin'` | `'Windows'` |
| `platform_machine` (x86-64) | `'x86_64'`  | `'x86_64'` | `'AMD64'`   |
| `platform_machine` (ARM64)  | `'aarch64'` | `'arm64'`  | `'ARM64'`   |
| `os_name`                   | `'posix'`   | `'posix'`  | `'nt'`      |

!!! note

    On Windows, `sys_platform` is always `'win32'`, even on 64-bit systems.

To check the values for the current platform, run:

```console
$ uvx python -c "import sysconfig; print(sysconfig.get_config_vars())"
```

## Dependency preferences

If `uv.lock` or `requirements.txt` already exists, uv _prefers_ its listed dependency versions. When
installing into a virtual environment, uv also prefers installed versions. These versions change
only if a requirement is incompatible or the command includes `--upgrade`.

## Resolution strategy

By default, uv selects the latest compatible version of each package. For example,
`uv pip install flask>=2.0.0` installs the latest Flask version, such as 3.0.0. Tests then run with
Flask 3.0.0 only. They do not verify compatibility with the declared lower bound of Flask 2.0.0.

With `--resolution lowest`, uv installs the lowest compatible version of every direct and transitive
dependency. With `--resolution lowest-direct`, uv selects the lowest compatible direct dependencies
and the latest compatible transitive dependencies. uv always uses the latest compatible build
dependencies.

For example, consider this `requirements.in` file:

```requirements title="requirements.in"
flask>=2.0.0
```

`uv pip compile requirements.in -o requirements.txt` produces this `requirements.txt` file:

```requirements title="requirements.txt"
# This file was autogenerated by uv via the following command:
#    uv pip compile requirements.in -o requirements.txt
blinker==1.7.0
    # via flask
click==8.1.7
    # via flask
flask==3.0.0
itsdangerous==2.1.2
    # via flask
jinja2==3.1.2
    # via flask
markupsafe==2.1.3
    # via
    #   jinja2
    #   werkzeug
werkzeug==3.0.1
    # via flask
```

In contrast, `uv pip compile --resolution lowest requirements.in -o requirements.txt` produces:

```requirements title="requirements.txt"
# This file was autogenerated by uv via the following command:
#    uv pip compile --resolution lowest requirements.in -o requirements.txt
click==7.1.2
    # via flask
flask==2.0.0
itsdangerous==2.0.0
    # via flask
jinja2==3.0.0
    # via flask
markupsafe==2.0.0
    # via jinja2
werkzeug==2.0.0
    # via flask
```

For published libraries, run separate continuous integration tests with `--resolution lowest` or
`--resolution lowest-direct`. These tests verify compatibility with declared lower bounds.

## Pre-release handling

By default (`if-necessary`), uv prefers stable versions. It considers pre-releases only after it
rejects every stable candidate that satisfies the active constraints.

Use `--prerelease allow` to consider pre-releases for every package without preferring stable
candidates first, or `--prerelease disallow` to exclude them entirely.

Use `--prerelease-package foo=allow` to override the global strategy for one package. Alternatively,
set package-specific strategies in `[tool.uv]`:

```toml
[tool.uv]
prerelease = "disallow"
prerelease-package = { foo = "allow", bar = "if-necessary" }
```

The `explicit` mode considers pre-releases only for first-party requirements with a pre-release
identifier. It prefers stable versions and uses pre-releases only when necessary. It rejects
pre-releases for all other packages.

For more details, see
[Pre-release compatibility](../pip/compatibility.md#pre-release-compatibility).

## Multi-version resolution

Universal resolution can list a package more than once in a lockfile. Different platforms or Python
versions might require different package versions or URLs.

The `--fork-strategy` setting controls whether uv prefers fewer package versions or newer versions
for each platform. Fewer versions improve consistency across platforms. Newer versions provide more
recent package releases when possible.

By default (`--fork-strategy requires-python`), uv selects the latest compatible package for each
supported Python version. It also minimizes the number of versions across platforms.

For example, with a Python requirement of `>=3.8`, uv selects these `numpy` versions:

```txt
numpy==1.24.4 ; python_version == "3.8"
numpy==2.0.2 ; python_version == "3.9"
numpy==2.2.0 ; python_version >= "3.10"
```

NumPy 2.2.0 and later require Python 3.10 or later. Older NumPy versions support Python 3.8 and 3.9.

With `--fork-strategy fewest`, uv minimizes the number of versions for each package. It prefers
older versions that support more Python versions or platforms.

In the previous example, uv selects `numpy==1.24.4` for all Python versions. It does not select
newer NumPy versions for Python 3.9 or later.

## Dependency constraints

Like pip, uv supports constraint files through `--constraint constraints.txt`. These files restrict
the acceptable versions of specific packages. Unlike requirements, a constraint does not add a
package to the resolution. It applies only when the package is already a direct or transitive
dependency. Constraints can restrict transitive dependency versions or keep shared packages aligned
with another resolution.

## Dependency overrides

Dependency overrides replace a package's declared dependencies to avoid failed or unwanted
resolutions. Use them only when a package is _known_ to be compatible despite its declared metadata.

For example, a transitive dependency might declare `pydantic>=1.0,<2.0` but still work with
`pydantic>=2.0`. Add `pydantic>=1.0,<3` as an override to let the resolver select a newer version.

An override of `pydantic>=1.0,<3` replaces every declared requirement on `pydantic`. In this
example, uv ignores `pydantic>=1.0,<2.0` and uses `pydantic>=1.0,<3` instead.

Constraints can only _reduce_ the set of acceptable package versions. Overrides can also _expand_
that set to work around incorrect upper bounds. Like constraints, global overrides do not add a
dependency. They apply only to existing direct or transitive dependencies.

In `pyproject.toml`, set `tool.uv.override-dependencies` to a list of overrides. In the
pip-compatible interface, use `--override` with files in the constraints format.

By default, an override applies to every requirement for the named dependency, including direct
requirements. Use an inline table to limit an override to one package version:

```toml
[tool.uv]
override-dependencies = [
    "foo>1",
    { package = { name = "bar", version = "0.0.5" }, dependencies = ["foo>2"] },
]
```

In this example, `foo>1` is the global override. `foo>2` replaces requirements for `foo` declared by
`bar==0.0.5`. If `bar` does not depend on `foo`, the scoped override adds that dependency. Other
dependencies of `bar` do not change. Omit the `version` field to apply a scoped override to every
version of `bar`. A version-specific entry takes precedence over an all-versions entry. A scoped
override takes precedence over a global override for the same dependency.

Scoped overrides support registry version specifiers only. They do not support direct URLs, paths,
Git sources, or explicit indexes.

In `explicit` pre-release mode, a pre-release specifier in any scoped override permits pre-release
fallback for that package throughout the resolution. Stable candidates remain preferred. Similarly,
an exact yanked-version pin permits yanked candidates throughout the resolution. This applies even
when uv does not select the override's scope.

Multiple overrides for the same package must use different [markers](#platform-markers). An override
replaces a dependency with a marker regardless of whether that marker evaluates to true or false.

## Dependency exclusions

Dependency exclusions remove packages from the dependency graph. By default, an exclusion applies to
every requirement for the named dependency, including direct requirements:

```toml
[tool.uv]
exclude-dependencies = ["foo"]
```

Use an inline table to limit an exclusion to one package version:

```toml
[tool.uv]
exclude-dependencies = [
    { package = { name = "bar", version = "0.0.5" }, dependencies = ["foo"] },
]
```

In this example, uv removes requirements for `foo` from `bar==0.0.5`. Requirements for `foo` from
other packages do not change. Omit the `version` field to apply the exclusion to every version of
`bar`. A version-specific entry takes precedence over an all-versions entry.

Combine scoped exclusions and overrides to replace one dependency with another:

```toml
[tool.uv]
override-dependencies = [
    { package = { name = "bar", version = "0.0.5" }, dependencies = ["pytorch-lightning"] },
]
exclude-dependencies = [
    { package = { name = "bar", version = "0.0.5" }, dependencies = ["lightning"] },
]
```

If the same dependency is both overridden and excluded in a matching scope, the exclusion takes
precedence.

## Dependency metadata

During resolution, uv reads each package's metadata to identify its dependencies. A package index
often provides this metadata as a static file. However, packages with only source distributions
might not provide metadata in advance.

If metadata is unavailable, uv must build the package to determine its dependencies. For example, it
might run `setup.py`. This slows resolution and requires the package to build on every platform.

For example, a Linux-only package might not build on macOS or Windows. A valid lockfile can still
include that package. However, building the package for its metadata fails on non-Linux platforms.

Use `tool.uv.dependency-metadata` to provide static metadata for these dependencies. uv can then
skip the build and use the supplied metadata.

For example, add `dependency-metadata` for `chumpy` to `pyproject.toml`:

```toml
[[tool.uv.dependency-metadata]]
name = "chumpy"
version = "0.70"
requires-dist = ["numpy>=1.8.1", "scipy>=0.13.0", "six>=1.11.0"]
```

These declarations help when a package does _not_ provide static metadata. They also help when a
package requires [disabling build isolation](./projects/config.md#build-isolation). Providing the
metadata can be easier than creating a custom build environment before resolution.

For example, older `flash-attn` versions did not declare static metadata. Provide that metadata so
uv can resolve `flash-attn` without building it or installing `torch` first:

```toml
[project]
name = "project"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["flash-attn"]

[tool.uv.sources]
flash-attn = { git = "https://github.com/Dao-AILab/flash-attention", tag = "v2.6.3" }

[[tool.uv.dependency-metadata]]
name = "flash-attn"
version = "2.6.3"
requires-dist = ["torch", "einops"]
```

`tool.uv.dependency-metadata` also helps when package metadata is incorrect or incomplete. It can
also provide metadata for a package that is absent from the index. Dependency overrides change
allowed package versions globally. Metadata overrides replace the declared metadata of a _specific
package_.

!!! note

    For registry dependencies, the `version` field is optional. Without it, uv applies the metadata
    to every version of the package. Direct URL dependencies, including Git dependencies,
    _require_ `version`.

Entries in `tool.uv.dependency-metadata` follow the
[Metadata 2.3](https://packaging.python.org/en/latest/specifications/core-metadata/) specification.
uv reads only `name`, `version`, `requires-dist`, `requires-python`, and `provides-extra`. If an
entry omits the optional `version` field, the metadata applies to every version of that package.

## Conflicting dependencies

When uv creates a lockfile, it resolves all project dependencies together. These dependencies must
be compatible. They include project dependencies, optional dependencies ("extras"), and development
dependency groups.

If two extras require incompatible dependencies, resolution fails. For example, these optional
dependencies conflict:

```toml title="pyproject.toml"
[project.optional-dependencies]
extra1 = ["numpy==2.1.2"]
extra2 = ["numpy==2.0.0"]
```

With these dependencies, `uv lock` fails:

```console
$ uv lock
  x No solution found when resolving dependencies:
  `-> Because myproject[extra2] depends on numpy==2.0.0 and myproject[extra1] depends on numpy==2.1.2, we can conclude that myproject[extra1] and
      myproject[extra2] are incompatible.
      And because your project requires myproject[extra1] and myproject[extra2], we can conclude that your projects's requirements are unsatisfiable.
```

Declare conflicts explicitly to resolve incompatible extras separately. Add the conflict to the
`tool.uv` section:

```toml title="pyproject.toml"
[tool.uv]
conflicts = [
    [
      { extra = "extra1" },
      { extra = "extra2" },
    ],
]
```

`uv lock` now succeeds. However, `extra1` and `extra2` cannot install together:

```console
$ uv sync --extra extra1 --extra extra2
Resolved 3 packages in 14ms
error: extra `extra1`, extra `extra2` are incompatible with the declared conflicts: {`myproject[extra1]`, `myproject[extra2]`}
```

Installing both extras would require two versions of the same package in one environment.

The same approach works for conflicting dependency groups:

```toml title="pyproject.toml"
[dependency-groups]
group1 = ["numpy==2.1.2"]
group2 = ["numpy==2.0.0"]

[tool.uv]
conflicts = [
    [
      { group = "group1" },
      { group = "group2" },
    ],
]
```

Use the `group` key instead of `extra`.

The same restrictions apply to workspaces with multiple projects. All workspace members must be
compatible unless a conflict is declared.

For example, consider the following workspace:

```toml title="member1/pyproject.toml"
[project]
name = "member1"

[project.optional-dependencies]
extra1 = ["numpy==2.1.2"]
```

```toml title="member2/pyproject.toml"
[project]
name = "member2"

[project.optional-dependencies]
extra2 = ["numpy==2.0.0"]
```

To declare a conflict between extras in different workspace members, use the `package` key:

```toml title="pyproject.toml"
[tool.uv]
conflicts = [
    [
      { package = "member1", extra = "extra1" },
      { package = "member2", extra = "extra2" },
    ],
]
```

The `project.dependencies` of one workspace member can also conflict with another member's extra:

```toml title="member1/pyproject.toml"
[project]
name = "member1"
dependencies = ["numpy==2.1.2"]
```

```toml title="member2/pyproject.toml"
[project]
name = "member2"

[project.optional-dependencies]
extra2 = ["numpy==2.0.0"]
```

Use the `package` key to declare this conflict:

```toml title="pyproject.toml"
[tool.uv]
conflicts = [
    [
      { package = "member1" },
      { package = "member2", extra = "extra2" },
    ],
]
```

Workspace members can also have conflicting project dependencies:

```toml title="member1/pyproject.toml"
[project]
name = "member1"
dependencies = ["numpy==2.1.2"]
```

```toml title="member2/pyproject.toml"
[project]
name = "member2"
dependencies = ["numpy==2.0.0"]
```

Use the `package` key to declare this conflict:

```toml title="pyproject.toml"
[tool.uv]
conflicts = [
    [
      { package = "member1" },
      { package = "member2" },
    ],
]
```

These workspace members cannot install together. For example, the workspace root cannot define:

```toml title="pyproject.toml"
[project]
name = "root"
dependencies = ["member1", "member2"]
```

## Lower bounds

By default, `uv add` adds lower bounds to dependencies. When managing a project, uv warns if a
direct dependency has no lower bound.

Lower bounds matter when dependencies conflict. For example, two required packages might have
incompatible dependencies. The resolver checks package versions within their allowed ranges. If all
combinations conflict, resolution fails. Without lower bounds, the resolver can backtrack to the
oldest available package versions. This slows resolution. Older versions might also fail to build or
remove the conflicting dependency while remaining incompatible with the project.

Libraries especially need accurate lower bounds. Declare the oldest compatible version of each
dependency. Test those bounds with
[`--resolution lowest` or `--resolution lowest-direct`](#resolution-strategy). Otherwise, users
might receive an incompatible dependency version and encounter unexpected errors.

## Reproducible resolutions

Use `--exclude-newer` to limit resolution to distributions uploaded before a specific date. This
makes installations reproducible even after new package releases. uv compares the cutoff with each
distribution file's upload time, not the package version's release date. Specify an
[RFC 3339](https://www.rfc-editor.org/rfc/rfc3339.html) timestamp, such as `2006-12-02T02:07:43Z`,
or a local date, such as `2006-12-02`. Local dates use the system's configured time zone.

!!! important

    The package index must support the [`PEP 700`](https://peps.python.org/pep-0700/) `upload-time`
    field. If a distribution lacks this field, uv treats it as unavailable. To exempt a package, use
    `--exclude-newer-package <package>=false`. An index can also set its own `exclude-newer` value or
    disable the cutoff with `[[tool.uv.index]] exclude-newer = false`. PyPI provides `upload-time`
    for every package.

To preserve reproducibility, error messages do not mention distributions excluded by
`--exclude-newer`. uv treats newer distributions as if they do not exist.

!!! note

    `--exclude-newer` applies only to packages from registries, not Git dependencies. In the
    `uv pip` interface, uv does not downgrade installed packages unless the command includes
    `--reinstall`. That option triggers a new resolution.

Set this option in `pyproject.toml` as follows:

```pyproject.toml
[tool.uv]
exclude-newer = "2006-12-02T02:07:43Z"
```

To disable a global cutoff from a lower-priority configuration source, pass `--exclude-newer false`,
set `UV_EXCLUDE_NEWER=false`, or set `exclude-newer = false` in a higher-priority configuration
file.

Persistent configuration does not accept local date times.

To set a package-specific cutoff, use `--exclude-newer-package setuptools=2006-12-02` or:

```pyproject.toml
[tool.uv]
exclude-newer-package = { setuptools = "2006-12-02T02:07:43Z" }
```

To exempt a package from the cutoff, use `--exclude-newer-package setuptools=false` or:

```pyproject.toml
[tool.uv]
exclude-newer-package = { setuptools = false }
```

This supports newer package versions and indexes that do not publish upload times.

Package-specific values take precedence over global and index-specific values.

An individual index can also override the global cutoff:

```pyproject.toml
[tool.uv]
exclude-newer = "2006-12-02T02:07:43Z"

[[tool.uv.index]]
name = "internal"
url = "https://internal.example.com/simple"
exclude-newer = "7 days"
```

To disable the cutoff for an index:

```pyproject.toml
[[tool.uv.index]]
name = "internal"
url = "https://internal.example.com/simple"
exclude-newer = false
```

This supports private indexes that do not publish `upload-time`. It also lets an index use a
different cutoff without changing the global setting.

## Dependency cooldowns

Dependency "cooldowns" exclude packages newer than a specified duration. They improve security by
delaying package updates until the community can review new versions.

Cooldowns use the same [`exclude-newer` option](#reproducible-resolutions) and follow the same
rules.

To define a cooldown, specify a duration instead of a date. Use a plain-language duration, such as
`24 hours`, `1 week`, or `30 days`. Alternatively, use an ISO 8601 duration, such as `PT24H`, `P7D`,
or `P30D`.

!!! note

    Durations use a fixed number of seconds and treat each day as 24 hours. They ignore local time
    zones and daylight saving transitions. Months and years are not valid because their lengths
    vary.

For a duration-based cutoff, uv calculates a timestamp relative to the current time. It stores that
timestamp in `uv.lock`. The timestamp does not change as time passes. uv updates it only during a
new resolution, such as one triggered by `--upgrade` or `--refresh`.

Set this option in `pyproject.toml` as follows:

```pyproject.toml
[tool.uv]
exclude-newer = "1 week"
```

To set a package-specific cooldown, use `--exclude-newer-package "setuptools=30 days"` or:

```pyproject.toml
[tool.uv]
exclude-newer = "1 week"
exclude-newer-package = { setuptools = "30 days" }
```

## Source distribution

[PEP 625](https://peps.python.org/pep-0625/) requires source distributions to use gzip tarball
(`.tar.gz`) archives. Older specifications also permitted other archive formats, which some tools
still support for backward compatibility.

!!! important

    Since version 0.12, uv rejects source distributions that do not conform to [PEP 625]'s extension
    requirements. It still accepts `.zip` archives for backward compatibility.

## Lockfile versioning

The `version` field in `uv.lock` identifies its lockfile schema version.

uv reads and writes lockfiles with a supported schema version. It rejects lockfiles with a newer
schema. For example, if uv supports schema v1, `uv lock` rejects a lockfile with schema v2.

A uv version that supports schema v2 _might_ also read schema v1 if the update was
backwards-compatible. However, uv can reject an outdated schema version.

The schema version is part of uv's public API. Breaking schema changes occur only in minor releases.
See [Versioning](../reference/policies/versioning.md). All patch versions within the same minor
release support the same lockfiles. A lockfile can become incompatible only across minor releases.

The `revision` field tracks backwards-compatible lockfile changes, such as a new distribution field.
Revision changes do not cause older uv versions to fail.

## Learn more

For more details about the internals of the resolver, see the
[resolver reference](../reference/internals/resolver.md) documentation.
