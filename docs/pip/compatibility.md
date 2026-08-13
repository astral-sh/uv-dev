# Compatibility with `pip` and `pip-tools`

uv replaces common `pip` and `pip-tools` workflows.

Most existing `pip` and `pip-tools` workflows work with uv without significant changes. In most
cases, replace `pip install` with `uv pip install`.

However, uv is _not_ an _exact_ clone of `pip`. Less common `pip` workflows are more likely to
behave differently. Some differences are intentional. Others result from implementation details or
bugs.

This document describes known differences between uv and `pip`. It explains their causes,
workarounds, and plans for future compatibility.

## Configuration files and environment variables

uv does not read configuration files or environment variables specific to `pip`, such as `pip.conf`
or `PIP_INDEX_URL`.

Reading configuration intended for other tools has several problems:

1. uv would need to match the other tool's bugs because workflows can depend on them.
2. uv would need to match any future changes to the other tool's configuration format.
3. For versioned configuration, uv would need to know the expected version of the other tool.
4. uv-specific settings could make shared configuration files incompatible with the original tool.
5. uv could read settings that do not affect its behavior or that are intended only for other tools.

Instead, uv uses its own environment variables, such as `UV_INDEX_URL`. It also supports persistent
configuration in `uv.toml` or the `[tool.uv.pip]` section of `pyproject.toml`. See
[Configuration files](../concepts/configuration-files.md) for more information.

## Pre-release compatibility

By default (`if-necessary`), uv prefers stable versions over pre-releases. It uses a pre-release
only when resolution rejects every stable candidate that meets the active constraints.

For example, suppose only `c==1.0` and `c==2.0a1` are available. The requirements `c>=1` and
`a -> c>=0.5a1` allow both versions. uv selects `c==1.0` regardless of which requirement it finds
first. If another active requirement rejects `c==1.0`, uv uses `c==2.0a1`.

Use `--prerelease allow` to consider pre-releases for every package without preferring stable
versions. Use `--prerelease disallow` to exclude pre-releases.

The `explicit` mode considers pre-releases only for first-party requirements with a pre-release
identifier. It prefers stable versions and uses pre-releases only when necessary. It rejects
pre-releases for all other packages.

!!! note

    Before pip 26.0, this behavior was not consistent.

Pre-releases are
[difficult to model](https://pubgrub-rs-guide.pages.dev/limitations/prerelease_versions) because
resolution discovers dependency requirements incrementally. uv keeps each package's candidate set
fixed and tries stable candidates before pre-releases. This lets backtracking reach a pre-release
without invalidating PubGrub's learned incompatibilities.

## Packages that exist on multiple indexes

Both uv and `pip` can search multiple package indexes for available package versions. However, they
handle packages that exist on multiple indexes differently.

For example, a company might publish an internal `requests` package on a private index
(`--extra-index-url`). It might also allow packages from PyPI by default. The private `requests`
package would conflict with the public [`requests`](https://pypi.org/project/requests/) package.

uv searches indexes in order and prefers `--extra-index-url` over the default index. It stops when
it finds a matching package. If the package exists on multiple indexes, uv considers versions only
from the first matching index.

`pip` combines candidate versions from all indexes and selects the best version from that set. It
[does not guarantee the search order](https://github.com/pypa/pip/issues/5045#issuecomment-369521345).
It also expects package names and versions to be unique across indexes.

If a package exists on an internal index, uv installs it from that index instead of PyPI. This
prevents "dependency confusion" attacks. In these attacks, a malicious PyPI package uses the same
name as an internal package and replaces it during installation. See
[the `torchtriton` attack](https://pytorch.org/blog/compromised-nightly-dependency/) from
December 2022.

Since v0.1.39, `--index-strategy` and `UV_INDEX_STRATEGY` can enable `pip`-style behavior for
multiple indexes. They support these values:

- `first-index` (default): Use versions only from the first index that contains the package. Search
  `--extra-index-url` indexes before the default index.
- `unsafe-first-match`: Use the first index with a compatible version, even if another index has a
  newer version.
- `unsafe-best-match`: Search all indexes and select the best version from all candidate versions.

`unsafe-best-match` most closely matches `pip`, but it creates a risk of "dependency confusion"
attacks.

uv can also pin a package to a dedicated index so that it _always_ installs from that index. See
[_Indexes_](../concepts/indexes.md#pinning-a-package-to-an-index).

## PEP 517 build isolation

uv uses [PEP 517](https://peps.python.org/pep-0517/) build isolation by default. This is similar to
`pip install --use-pep517` and follows `pypa/build`. `pip` also plans to enable PEP 517 builds by
default ([pypa/pip#9175](https://github.com/pypa/pip/issues/9175)).

If installation fails because a build-time dependency is missing, try a newer package version. If
the problem continues, ask the package maintainer to declare the required PEP 517 build
dependencies.

Alternatively, install the package's build dependencies first. Then run `uv pip install` with
`--no-build-isolation`:

```shell
uv pip install wheel && uv pip install --no-build-isolation biopython==1.77
```

For packages that fail under PEP 517 build isolation, see
[#2252](https://github.com/astral-sh/uv/issues/2252).

## Transitive URL dependencies

uv supports URL dependencies, such as `ruff @ https://...`. Its handling of _transitive_ URL
dependencies differs from pip in two ways.

First, uv assumes that packages from registries do not depend on URLs. If a non-URL dependency
introduces a URL dependency, uv rejects that URL dependency during resolution. PyPI does not allow
published packages to depend on URLs, but other registries can allow them.

Second, a constraint (`--constraint`) or override (`--override`) can specify a direct URL
dependency. If the constrained package also has a direct URL dependency, uv _may_ reject that
transitive URL. This can happen when the input requirements do not reference the URL elsewhere.

If uv rejects a transitive URL dependency, add it directly to `pyproject.toml` or `requirements.in`.
These restrictions do not apply to direct dependencies.

## Virtual environments by default

`uv pip install` and `uv pip sync` use virtual environments by default.

uv installs packages into the active virtual environment. If no virtual environment is active, uv
searches for `.venv` in the current directory and its parent directories.

In contrast, `pip` installs packages into a global environment when no virtual environment is
active. It does not search for inactive virtual environments.

Use `--python /path/to/python` to install packages into a non-virtual environment. Alternatively,
use `--system` to install into the first Python interpreter on `PATH`, as `pip` does.

uv requires an explicit option to install packages into the system Python environment. System
installations can break the environment, so use them only when necessary.

For more information, see
["Using arbitrary Python environments"](./environments.md#using-arbitrary-python-environments).

## Resolution strategy

A set of dependency specifiers often has multiple valid package resolutions.

Neither `pip` nor uv guarantees the _exact_ set of installed packages. Each resolution must be
consistent, deterministic, and compatible with the specifiers. The tools can produce different
resolutions, but both _should_ be valid.

For example, consider:

```requirements title="requirements.in"
starlette
fastapi
```

In this example, the most recent `starlette` version is `0.37.2`, and the most recent `fastapi`
version is `0.110.0`. However, `fastapi==0.110.0` requires `starlette>=0.36.3,<0.37.0`.

If a resolver prefers the newest `starlette` version, it must use an older `fastapi` version without
that upper bound. This requires `fastapi==0.1.17`:

```requirements title="requirements.txt"
# This file was autogenerated by uv via the following command:
#    uv pip compile requirements.in
annotated-types==0.6.0
    # via pydantic
anyio==4.3.0
    # via starlette
fastapi==0.1.17
idna==3.6
    # via anyio
pydantic==2.6.3
    # via fastapi
pydantic-core==2.16.3
    # via pydantic
sniffio==1.3.1
    # via anyio
starlette==0.37.2
    # via fastapi
typing-extensions==4.10.0
    # via
    #   pydantic
    #   pydantic-core
```

If a resolver instead prefers the newest `fastapi` version, it must use an older `starlette` version
that meets the upper bound. This requires `starlette==0.36.3`:

```requirements title="requirements.txt"
# This file was autogenerated by uv via the following command:
#    uv pip compile requirements.in
annotated-types==0.6.0
    # via pydantic
anyio==4.3.0
    # via starlette
fastapi==0.110.0
idna==3.6
    # via anyio
pydantic==2.6.3
    # via fastapi
pydantic-core==2.16.3
    # via pydantic
sniffio==1.3.1
    # via anyio
starlette==0.36.3
    # via fastapi
typing-extensions==4.10.0
    # via
    #   fastapi
    #   pydantic
    #   pydantic-core
```

If uv produces an unwanted resolution that differs from `pip`, use more specific requirements. For
example, require `fastapi>=0.110.0`.

## `pip check`

`uv pip check` reports these problems:

- A package has no `METADATA` file, or its `METADATA` file cannot be parsed.
- A package has a `Requires-Python` value that does not match the Python version of the running
  interpreter.
- A package depends on a package that is not installed.
- A package depends on an installed package with an incompatible version.
- Multiple versions of a package are installed in the virtual environment.

`uv pip check` and `pip check` do not report all the same problems. For example, `pip check` does
_not_ warn when multiple versions of a package exist in the current environment.

## `--user` and the `user` install scheme

uv does not support the `--user` flag, which installs packages with the `user` install scheme. Use
virtual environments to isolate package installations instead.

pip also uses the `user` install scheme when the target directory is not writable. This can happen
when installing packages into the system Python environment. uv does not provide this fallback.

For more information, see [#2077](https://github.com/astral-sh/uv/issues/2077).

## `--only-binary` enforcement

The `--only-binary` argument restricts installation to pre-built binary distributions. With
`--only-binary :all:`, both pip and uv reject source distributions from PyPI and other registries.

However, pip does _not_ enforce `--only-binary` for direct URL dependencies, such as
`uv pip install https://...`. It builds source distributions for those packages.

uv _does_ enforce `--only-binary` for direct URL dependencies, with one exception. Consider
`uv pip install https://... --only-binary flask`. If uv cannot infer the package name, it _will_
build the source distribution to read its metadata. Without the name, uv cannot determine whether
the binary-only restriction applies.

Both pip and uv allow editable requirements with `--only-binary`. For example,
`uv pip install -e . --only-binary :all:` is valid.

## `--no-binary` enforcement

The `--no-binary` argument restricts installation to source distributions. With `--no-binary`, uv
does not install pre-built binary distributions. However, it _does_ reuse binary distributions
already in the local cache.

Unlike pip, uv still reads metadata from pre-built binary distributions with `--no-binary`.

## `manylinux_compatible` enforcement

[PEP 600](https://peps.python.org/pep-0600/#package-installers) lets Python distributors disable
`manylinux` compatibility. They do this by defining a `manylinux_compatible` function on the
`_manylinux` standard library module.

uv respects `manylinux_compatible`, but tests only the current glibc version. It applies the result
globally.

If `manylinux_compatible` returns `True`, uv treats the system as `manylinux`-compatible. If it
returns `False`, uv treats the system as `manylinux`-incompatible. uv does not call the function for
every glibc version.

This does not fully implement the specification. However, it supports common system-wide
`manylinux_compatible` implementations such as
[`no-manylinux`](https://pypi.org/project/no-manylinux/):

```python
from __future__ import annotations

manylinux1_compatible = False
manylinux2010_compatible = False
manylinux2014_compatible = False


def manylinux_compatible(*_, **__):  # PEP 600
    return False
```

## Bytecode compilation

Unlike `pip`, uv does not compile `.py` files to `.pyc` files during installation by default. It
does not create or populate `__pycache__` directories. To enable bytecode compilation, use
`--compile-bytecode` with `uv pip install` or `uv pip sync`. Alternatively, set
`UV_COMPILE_BYTECODE=1`.

Some workflows benefit from bytecode compilation. For example, enable it in
[Docker builds](../guides/integration/docker.md) to reduce startup time. This increases build time.

Bytecode compilation suppresses some Python interpreter warnings. Without it, code installed with uv
can produce `SyntaxWarning` or `DeprecationWarning` messages that pip installations do not show.
These warnings are valid. Ignore them, fix their cause in the package, or enable bytecode
compilation to suppress them.

## Strictness and spec enforcement

uv is often stricter than `pip` and can reject packages that `pip` accepts. For example, uv rejects
HTML indexes with invalid URL fragments under [PEP 503](https://peps.python.org/pep-0503/). `pip`
ignores those fragments.

uv makes exceptions for some popular packages with known specification compliance issues.

If uv rejects a package because it violates a specification, try a newer version. If that fails,
report the problem to the package maintainer.

## `pip` command-line options and subcommands

uv supports many, but not all, `pip` command-line options and subcommands.

uv prioritizes missing options and subcommands by demand and implementation complexity. Individual
issues track them. For example:

- [`--trusted-host`](https://github.com/astral-sh/uv/issues/1339)
- [`--user`](https://github.com/astral-sh/uv/issues/2077)

If an option or subcommand is missing, search the issue tracker for an existing report. Open an
issue if no report exists, or upvote an existing issue.

## Registry authentication

uv does not support the `auto` or `import` values for `--keyring-provider`. It supports only
`subprocess`.

Unlike `pip`, uv does not enable keyring authentication by default.

Unlike `pip`, uv does not wait for an HTTP 401 response before searching for credentials. It adds
authentication to all requests for hosts with available credentials.

## `egg` support

uv does not support legacy or deprecated `pip` features, such as `.egg` distributions.

However, uv partially supports `.egg-info` distributions, which can occur in Docker images and Conda
environments. It also partially supports legacy editable `.egg-link` distributions.

uv cannot install new `.egg-info` or `.egg-link` distributions. However, it recognizes existing
distributions during resolution. It can also list them with `uv pip list` or `uv pip freeze`, and
remove them with `uv pip uninstall`.

## Build constraints

uv does _not_ apply `--constraint` or `UV_CONSTRAINT` to build dependencies. Use
`--build-constraint` or `UV_BUILD_CONSTRAINT` for build dependencies instead.

pip applies `PIP_CONSTRAINT` to build dependencies. It does not apply command-line `--constraint`
values to build dependencies.

For example, use `--build-constraint` to require `setuptools 60.0.0` for all packages that need
`setuptools` to build.

## `pip compile` defaults

The default behavior of `uv pip compile` differs from `pip-tools` in several ways.

By default, uv does not write compiled requirements to a file. Use `-o` or `--output-file` to
specify an output file.

By default, uv removes extras from compiled requirements. uv defaults to `--strip-extras`, while
`pip-compile` defaults to `--no-strip-extras`. `pip-compile` plans to use `--strip-extras` by
default in its next major release, v8.0.0. To keep extras with uv, use
`uv pip compile --no-strip-extras`.

By default, uv does not write index URLs to the output file. `pip-compile` includes `--index-url`
and `--extra-index-url` values that differ from the default PyPI index. Use
`uv pip compile --emit-index-url` to include index URLs. Unlike `pip-compile`, uv includes every
index URL, including the default.

## `requires-python` upper bounds

For dependency `requires-python` ranges, uv considers lower bounds and ignores upper bounds. For
example, uv treats `>=3.8, <4` as `>=3.8`. Respecting upper bounds can produce formally correct but
impractical resolutions. For example, a resolver can select an old package version without the upper
bound. See
[`Requires-Python` upper limits](https://discuss.python.org/t/requires-python-upper-limits/12663).

## `requires-python` specifiers

When comparing Python versions with `requires-python` specifiers, uv uses only the major, minor, and
patch components. It ignores identifiers such as pre-releases and post-releases.

For example, a project with `requires-python: >=3.13` accepts Python 3.13.0b1. This pre-release does
not strictly meet the requirement. It does meet the requirement after uv removes the pre-release
identifier.

This does not strictly comply with [PEP 440](https://peps.python.org/pep-0440/), but it _does_ match
[pip](https://github.com/pypa/pip/blob/24.1.1/src/pip/_internal/resolution/resolvelib/candidates.py#L540).

## Package priority

A set of requirements usually has multiple valid resolutions. uv and pip use different package
priorities to select one. Both consider the order of the specified requirements, but pip has
additional
[priorities](https://pip.pypa.io/en/stable/topics/more-dependency-resolution/#the-resolver-algorithm).
As a result, uv is more likely than pip to produce different results when the requirement order
changes.

For example, `uv pip install foo bar` prefers newer versions of `foo` over `bar`. It can produce a
different resolution from `uv pip install bar foo`. Requirement order also matters in input files
for `uv pip compile`.

## Wheel filename and metadata validation

By default, uv rejects wheels when the filename does not match the included wheel metadata. For
example, consider a wheel named `foo-1.0.0-py3-none-any.whl` with metadata for version `1.0.1`. uv
rejects this wheel, but pip accepts it.

Set `UV_SKIP_WHEEL_FILENAME_CHECK=1` to make uv accept these wheels.

## Package name normalization

By default, uv normalizes package names to their
[PEP 503-compliant forms](https://packaging.python.org/en/latest/specifications/name-normalization/#name-normalization)
and uses the normalized names in all output. pip generally preserves the package name published on
the registry.

For example, `uv pip list` displays normalized package names, such as `docstring-parser`. `pip list`
displays non-normalized names, such as `docstring_parser`:

```shell
(venv) $ diff --side-by-side  <(pip list) <(uv pip list)
Package          Version					Package          Version
---------------- -------					---------------- -------
docstring_parser 0.16					      |	docstring-parser 0.16
jaraco.classes   3.4.0					      |	jaraco-classes   3.4.0
more-itertools   10.7.0				    		more-itertools   10.7.0
pip              25.1					    	pip              25.1
PyMuPDFb         1.24.10				      |	pymupdfb         1.24.10
PyPDF2           3.0.1					      |	pypdf2           3.0.1
```
