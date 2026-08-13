# Managing dependencies

## Dependency fields

Project dependencies are defined in several fields:

- [`project.dependencies`](#project-dependencies): Published dependencies.
- [`project.optional-dependencies`](#optional-dependencies): Published optional dependencies, or
  "extras".
- [`dependency-groups`](#dependency-groups): Local dependencies for development.
- [`tool.uv.sources`](#dependency-sources): Alternative sources for dependencies during development.

!!! note

    The `project.dependencies` and `project.optional-dependencies` fields also support projects
    that will not be published. Dependency groups are a recently standardized feature that some
    tools may not yet support.

`uv add` and `uv remove` modify project dependencies. Editing `pyproject.toml` directly also changes
dependency metadata.

## Adding dependencies

The following command adds a dependency:

```console
$ uv add httpx
```

uv adds an entry to the `project.dependencies` field:

```toml title="pyproject.toml" hl_lines="4"
[project]
name = "example"
version = "0.1.0"
dependencies = ["httpx>=0.27.2"]
```

The [`--dev`](#development-dependencies), [`--group`](#dependency-groups), and
[`--optional`](#optional-dependencies) options add dependencies to other fields.

uv adds a version constraint, such as `>=0.27.2`, for the most recent compatible package version.
The [`--bounds`](../../reference/settings.md#add-bounds) option changes the type of bound. A
constraint can also be specified directly:

```console
$ uv add "httpx>=0.20"
```

When a dependency does not come from a package registry, uv adds an entry to the sources field. For
example, the following command adds `httpx` from GitHub:

```console
$ uv add "httpx @ git+https://github.com/encode/httpx"
```

uv adds a [Git source entry](#git) to `pyproject.toml`:

```toml title="pyproject.toml" hl_lines="8-9"
[project]
name = "example"
version = "0.1.0"
dependencies = [
    "httpx",
]

[tool.uv.sources]
httpx = { git = "https://github.com/encode/httpx" }
```

If a dependency cannot be used, uv displays an error:

```console
$ uv add "httpx>9999"
  × No solution found when resolving dependencies:
  ╰─▶ Because only httpx<=1.0.0b0 is available and your project depends on httpx>9999,
      we can conclude that your project's requirements are unsatisfiable.
```

### Importing dependencies from requirements files

The `-r` option adds dependencies from a `requirements.txt` file to the project:

```
uv add -r requirements.txt
```

The [pip migration guide](../../guides/migration/pip-to-project.md#importing-requirements-files)
provides more details.

## Removing dependencies

The following command removes a dependency:

```console
$ uv remove httpx
```

The `--dev`, `--group`, and `--optional` options remove dependencies from specific tables.

uv also removes the dependency's [source](#dependency-sources) if no other dependency refers to it.

## Changing dependencies

The following command changes an existing dependency constraint:

```console
$ uv add "httpx>0.1.0"
```

!!! note

    This example changes the dependency constraints in `pyproject.toml`. The locked version changes
    only if needed to satisfy the new constraints. The `--upgrade-package <name>` option updates the
    package to the latest version that satisfies those constraints:

    ```console
    $ uv add "httpx>0.1.0" --upgrade-package httpx
    ```

    The [lockfile](./sync.md#upgrading-locked-package-versions) documentation provides more details
    about package upgrades.

Requesting a different dependency source updates `tool.uv.sources`. For example, the following
command uses a local `httpx` checkout during development:

```console
$ uv add "httpx @ ../httpx"
```

## Platform-specific dependencies

[Environment markers](https://peps.python.org/pep-0508/#environment-markers) limit a dependency to
specific platforms or Python versions.

For example, the following command installs `jax` on Linux but not on Windows or macOS:

```console
$ uv add "jax; sys_platform == 'linux'"
```

The resulting `pyproject.toml` includes the environment marker in the dependency definition:

```toml title="pyproject.toml" hl_lines="6"
[project]
name = "project"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = ["jax; sys_platform == 'linux'"]
```

The following command includes `numpy` on Python 3.11 and later:

```console
$ uv add "numpy; python_version >= '3.11'"
```

Python's [environment marker](https://peps.python.org/pep-0508/#environment-markers) documentation
lists the available markers and operators.

!!! tip

    Dependency sources can also [depend on the platform](#platform-specific-sources).

## Project dependencies

The `project.dependencies` table lists the dependencies included in PyPI uploads and built wheels.
Each dependency uses
[dependency specifier](https://packaging.python.org/en/latest/specifications/dependency-specifiers/)
syntax. The table follows the
[PEP 621](https://packaging.python.org/en/latest/specifications/pyproject-toml/) standard.

`project.dependencies` lists the packages that the project requires and their version constraints.
Each entry includes a package name and version. An entry may also include extras or platform
markers. For example:

```toml title="pyproject.toml"
[project]
name = "albatross"
version = "0.1.0"
dependencies = [
  # Any version in this range
  "tqdm >=4.66.2,<5",
  # Exactly this version of torch
  "torch ==2.2.2",
  # Install transformers with the torch extra
  "transformers[torch] >=4.39.3,<5",
  # Only install this package on older python versions
  # See "Environment Markers" for more information
  "importlib_metadata >=7.1.0,<8; python_version < '3.10'",
  "mollymawk ==0.1.0"
]
```

## Dependency sources

The `tool.uv.sources` table adds alternative development sources to standard dependency definitions.

Dependency sources support patterns that `project.dependencies` does not, such as editable
installations and relative paths. For example, the following configuration installs `foo` from a
directory relative to the project root:

```toml title="pyproject.toml" hl_lines="7"
[project]
name = "example"
version = "0.1.0"
dependencies = ["foo"]

[tool.uv.sources]
foo = { path = "./packages/foo" }
```

uv supports these dependency sources:

- [Index](#index): A package resolved from a specific package index.
- [Git](#git): A Git repository.
- [URL](#url): A remote wheel or source distribution.
- [Path](#path): A local wheel, source distribution, or project directory.
- [Workspace](#workspace-member): A member of the current workspace.

!!! important

    Only uv uses these sources. Other tools use the standard project tables. To use another
    development tool, its configuration must include any required source metadata in its own
    format.

### Index

The `--index` option adds a Python package from a specific index:

```console
$ uv add torch --index pytorch=https://download.pytorch.org/whl/cpu
```

uv stores the index in `[[tool.uv.index]]` and adds a `[tool.uv.sources]` entry:

```toml title="pyproject.toml"
[project]
dependencies = ["torch"]

[tool.uv.sources]
torch = { index = "pytorch" }

[[tool.uv.index]]
name = "pytorch"
url = "https://download.pytorch.org/whl/cpu"
```

If the index is already configured, you can select it by name (this feature is in preview):

```console
$ uv add --preview-features index-by-name torch --index pytorch
```

!!! tip

    This example works only on x86-64 Linux because of the PyTorch index configuration. The
    [PyTorch guide](../../guides/integration/pytorch.md) provides more setup information.

An `index` source _pins_ a package to that index. uv does not download the package from other
indexes.

The `explicit` field limits an index to packages that name it in `tool.uv.sources`. Without
`explicit`, uv may resolve other packages from the index if they are not available elsewhere.

```toml title="pyproject.toml" hl_lines="4"
[[tool.uv.index]]
name = "pytorch"
url = "https://download.pytorch.org/whl/cpu"
explicit = true
```

### Git

A Git dependency source uses a Git-compatible URL with the `git+` prefix.

For example:

```console
$ # Install over HTTP(S).
$ uv add git+https://github.com/encode/httpx

$ # Install over SSH.
$ uv add git+ssh://git@github.com/encode/httpx
```

```toml title="pyproject.toml" hl_lines="5"
[project]
dependencies = ["httpx"]

[tool.uv.sources]
httpx = { git = "https://github.com/encode/httpx" }
```

The `--tag` option selects a specific Git tag:

```console
$ uv add git+https://github.com/encode/httpx --tag 0.27.0
```

```toml title="pyproject.toml" hl_lines="7"
[project]
dependencies = ["httpx"]

[tool.uv.sources]
httpx = { git = "https://github.com/encode/httpx", tag = "0.27.0" }
```

The `--branch` option selects a branch:

```console
$ uv add git+https://github.com/encode/httpx --branch main
```

```toml title="pyproject.toml" hl_lines="7"
[project]
dependencies = ["httpx"]

[tool.uv.sources]
httpx = { git = "https://github.com/encode/httpx", branch = "main" }
```

The `--rev` option selects a revision, or commit:

```console
$ uv add git+https://github.com/encode/httpx --rev 326b9431c761e1ef1e00b9f760d1f654c8db48c6
```

```toml title="pyproject.toml" hl_lines="7"
[project]
dependencies = ["httpx"]

[tool.uv.sources]
httpx = { git = "https://github.com/encode/httpx", rev = "326b9431c761e1ef1e00b9f760d1f654c8db48c6" }
```

The `subdirectory` field identifies a package that is not in the repository root:

```console
$ uv add git+https://github.com/langchain-ai/langchain#subdirectory=libs/langchain
```

```toml title="pyproject.toml"
[project]
dependencies = ["langchain"]

[tool.uv.sources]
langchain = { git = "https://github.com/langchain-ai/langchain", subdirectory = "libs/langchain" }
```

[Git LFS](https://git-lfs.com) support can be configured for each source. By default, uv does not
fetch Git LFS objects.

```console
$ uv add --lfs git+https://github.com/astral-sh/lfs-cowsay
```

```toml title="pyproject.toml"
[project]
dependencies = ["lfs-cowsay"]

[tool.uv.sources]
lfs-cowsay = { git = "https://github.com/astral-sh/lfs-cowsay", lfs = true }
```

- When `lfs = true`, uv always fetches LFS objects for this Git source.
- When `lfs = false`, uv never fetches LFS objects for this Git source.
- If `lfs` is omitted, the `UV_GIT_LFS` environment variable controls Git LFS for the source.

!!! important

    Git LFS must be installed and configured before uv installs sources that use it. Otherwise, the
    build can fail.

### URL

A URL source uses an `https://` URL for a wheel or source distribution. Wheel names end in `.whl`.
Source distributions usually end in `.tar.gz` or `.zip`. The
[source distribution documentation](../../concepts/resolution.md#source-distribution) lists all
supported formats.

For example:

```console
$ uv add "https://files.pythonhosted.org/packages/5c/2d/3da5bdf4408b8b2800061c339f240c1802f2e82d55e50bd39c5a881f47f0/httpx-0.27.0.tar.gz"
```

uv adds the following entry to `pyproject.toml`:

```toml title="pyproject.toml" hl_lines="5"
[project]
dependencies = ["httpx"]

[tool.uv.sources]
httpx = { url = "https://files.pythonhosted.org/packages/5c/2d/3da5bdf4408b8b2800061c339f240c1802f2e82d55e50bd39c5a881f47f0/httpx-0.27.0.tar.gz" }
```

URL dependencies also support the `{ url = <url> }` syntax in `pyproject.toml`. The `subdirectory`
field identifies a source distribution outside the archive root.

### Path

A path source can point to a wheel, a source distribution, or a directory that contains
`pyproject.toml`. Wheels end in `.whl`. Source distributions usually end in `.tar.gz` or `.zip`. The
[source distribution documentation](../../concepts/resolution.md#source-distribution) lists all
supported formats.

For example:

```console
$ uv add /example/foo-0.1.0-py3-none-any.whl
```

uv adds the following entry to `pyproject.toml`:

```toml title="pyproject.toml"
[project]
dependencies = ["foo"]

[tool.uv.sources]
foo = { path = "/example/foo-0.1.0-py3-none-any.whl" }
```

The path can also be relative:

```console
$ uv add ./foo-0.1.0-py3-none-any.whl
```

It can also point to a project directory:

```console
$ uv add ~/projects/bar/
```

!!! important

    By default, uv attempts to build and install directory dependencies as packages. The
    [virtual dependency](#virtual-dependencies) documentation provides more details.

Path dependencies are not [editable installations](#editable-dependencies) by default. The
`--editable` option requests an editable installation for a project directory:

```console
$ uv add --editable ../projects/bar/
```

uv adds the following entry to `pyproject.toml`:

```toml title="pyproject.toml"
[project]
dependencies = ["bar"]

[tool.uv.sources]
bar = { path = "../projects/bar", editable = true }
```

!!! tip

    [_Workspaces_](./workspaces.md) may work better for multiple packages in one repository.

### Workspace member

A dependency on a workspace member uses the member name and `{ workspace = true }`. All workspace
dependencies must explicitly declare their source. Workspace members are always
[editable](#editable-dependencies). The [workspace](./workspaces.md) documentation provides more
details.

The `workspace` field also accepts a path to another workspace:

```toml title="pyproject.toml"
[tool.uv.sources]
foo = { workspace = "../other-workspace" }
```

```toml title="pyproject.toml"
[project]
dependencies = ["foo==0.1.0"]

[tool.uv.sources]
foo = { workspace = true }

[tool.uv.workspace]
members = [
  "packages/foo"
]
```

### Platform-specific sources

[Dependency specifier](https://packaging.python.org/en/latest/specifications/dependency-specifiers/)-compatible
environment markers can limit a source to a specific platform or Python version.

For example, this configuration downloads `httpx` from GitHub only on macOS:

```toml title="pyproject.toml" hl_lines="8"
[project]
dependencies = ["httpx"]

[tool.uv.sources]
httpx = { git = "https://github.com/encode/httpx", tag = "0.27.2", marker = "sys_platform == 'darwin'" }
```

The source marker does not remove `httpx` from other platforms. uv downloads it from GitHub on macOS
and from PyPI on other platforms.

### Multiple sources

A dependency can have multiple sources. Each source uses a
[PEP 508](https://peps.python.org/pep-0508/#environment-markers)-compatible environment marker to
identify where it applies.

For example, this configuration selects different `httpx` tags on macOS and Linux:

```toml title="pyproject.toml" hl_lines="6-7"
[project]
dependencies = ["httpx"]

[tool.uv.sources]
httpx = [
  { git = "https://github.com/encode/httpx", tag = "0.27.2", marker = "sys_platform == 'darwin'" },
  { git = "https://github.com/encode/httpx", tag = "0.24.1", marker = "sys_platform == 'linux'" },
]
```

Environment markers can also select different indexes. For example, this configuration selects a
PyTorch index for each platform:

```toml title="pyproject.toml" hl_lines="6-7"
[project]
dependencies = ["torch"]

[tool.uv.sources]
torch = [
  { index = "torch-cpu", marker = "platform_system == 'Darwin'"},
  { index = "torch-gpu", marker = "platform_system == 'Linux'"},
]

[[tool.uv.index]]
name = "torch-cpu"
url = "https://download.pytorch.org/whl/cpu"
explicit = true

[[tool.uv.index]]
name = "torch-gpu"
url = "https://download.pytorch.org/whl/cu130"
explicit = true
```

### Disabling sources

The `--no-sources` option makes uv ignore `tool.uv.sources`. This can simulate resolution with a
package's published metadata:

```console
$ uv lock --no-sources
```

The `--no-sources` option also prevents uv from discovering [workspace members](#workspace-member)
that could satisfy a dependency.

## Optional dependencies

Published libraries often make features optional to reduce their default dependencies. For example,
Pandas provides an
[`excel` extra](https://pandas.pydata.org/docs/getting_started/install.html#excel-files) and a
[`plot` extra](https://pandas.pydata.org/docs/getting_started/install.html#visualization). Excel
parsers and `matplotlib` are installed only when the relevant extra is requested. The
`package[<extra>]` syntax requests extras, such as `pandas[plot, excel]`.

The `[project.optional-dependencies]` table maps each extra name to its dependencies. Entries use
[dependency specifier](#dependency-specifiers) syntax.

Optional dependencies can have `tool.uv.sources` entries, just like other dependencies.

```toml title="pyproject.toml"
[project]
name = "pandas"
version = "1.0.0"

[project.optional-dependencies]
plot = [
  "matplotlib>=3.6.3"
]
excel = [
  "odfpy>=1.4.1",
  "openpyxl>=3.1.0",
  "python-calamine>=0.1.7",
  "pyxlsb>=1.0.10",
  "xlrd>=2.0.1",
  "xlsxwriter>=3.0.5"
]
```

The `--optional <extra>` option adds an optional dependency:

```console
$ uv add httpx --optional network
```

!!! note

    Resolution fails when optional dependencies conflict unless the configuration
    [explicitly declares the conflict](./config.md#conflicting-dependencies).

A source can also apply only to a specific optional dependency. For example, this configuration
selects different PyTorch indexes for the optional `cpu` and `gpu` extras:

```toml title="pyproject.toml"
[project]
dependencies = []

[project.optional-dependencies]
cpu = [
  "torch",
]
gpu = [
  "torch",
]

[tool.uv.sources]
torch = [
  { index = "torch-cpu", extra = "cpu" },
  { index = "torch-gpu", extra = "gpu" },
]

[[tool.uv.index]]
name = "torch-cpu"
url = "https://download.pytorch.org/whl/cpu"

[[tool.uv.index]]
name = "torch-gpu"
url = "https://download.pytorch.org/whl/cu130"
```

## Development dependencies

Unlike optional dependencies, development dependencies are local. Published project requirements _do
not_ include them, so they do not appear in the `[project]` table.

Development dependencies can have `tool.uv.sources` entries, just like other dependencies.

The `--dev` option adds a development dependency:

```console
$ uv add --dev pytest
```

uv stores development dependencies in the `[dependency-groups]` table defined by
[PEP 735](https://peps.python.org/pep-0735/). This command creates a `dev` group:

```toml title="pyproject.toml"
[dependency-groups]
dev = [
  "pytest >=8.1.1,<9"
]
```

The `--dev`, `--only-dev`, and `--no-dev` options control the `dev` group. The `--no-default-groups`
option disables all default groups. uv [syncs the `dev` group by default](#default-groups).

### Dependency groups

The `--group` option assigns development dependencies to different groups.

For example, the following command adds a development dependency to the `lint` group:

```console
$ uv add --group lint ruff
```

The command creates this `[dependency-groups]` definition:

```toml title="pyproject.toml"
[dependency-groups]
dev = [
  "pytest"
]
lint = [
  "ruff"
]
```

The `--all-groups`, `--no-default-groups`, `--group`, `--only-group`, and `--no-group` options
include or exclude group dependencies.

!!! tip

    The `--dev`, `--only-dev`, and `--no-dev` options are equivalent to `--group dev`,
    `--only-group dev`, and `--no-group dev`, respectively.

uv resolves all dependency groups together when it creates the lockfile. Groups must be compatible
unless their conflict is explicitly declared.

!!! note

    Resolution fails when dependency groups conflict unless the configuration
    [explicitly declares the conflict](./config.md#conflicting-dependencies).

### Nesting groups

A dependency group can include other dependency groups:

```toml title="pyproject.toml"
[dependency-groups]
dev = [
  {include-group = "lint"},
  {include-group = "test"}
]
lint = [
  "ruff"
]
test = [
  "pytest"
]
```

An included group's dependencies must not conflict with the other dependencies in the parent group.

### Default groups

By default, uv includes the `dev` dependency group during commands such as `uv run` and `uv sync`.
The `tool.uv.default-groups` setting changes the default groups.

```toml title="pyproject.toml"
[tool.uv]
default-groups = ["dev", "foo"]
```

The value `"all"` includes every dependency group by default:

```toml title="pyproject.toml"
[tool.uv]
default-groups = "all"
```

!!! tip

    The `--no-default-groups` option disables default groups during `uv run` or `uv sync`. The
    `--no-group <name>` option excludes a specific default group.

### Group `requires-python`

By default, dependency groups must support the project's `requires-python` range.

If a group requires a different Python version range, `[tool.uv.dependency-groups]` can specify a
separate `requires-python` value:

```toml title="pyproject.toml" hl_lines="9-10"
[project]
name = "example"
version = "0.0.0"
requires-python = ">=3.10"

[dependency-groups]
dev = ["pytest"]

[tool.uv.dependency-groups]
dev = {requires-python = ">=3.12"}
```

### Legacy `dev-dependencies`

Before `[dependency-groups]` was standardized, uv stored development dependencies in
`tool.uv.dev-dependencies`:

```toml title="pyproject.toml"
[tool.uv]
dev-dependencies = [
  "pytest"
]
```

uv combines dependencies from this field with `dependency-groups.dev`. The `dev-dependencies` field
will eventually be deprecated and removed.

!!! note

    If `tool.uv.dev-dependencies` exists, `uv add --dev` uses that field instead of creating
    `dependency-groups.dev`.

## Build dependencies

A [Python package](./config.md#build-systems) can declare dependencies needed only to build it. The
`build-system.requires` field in `[build-system]` lists these dependencies, as defined by
[PEP 518](https://peps.python.org/pep-0518/).

For example, a project that uses the `setuptools` build backend should declare `setuptools` as a
build dependency:

```toml title="pyproject.toml"
[project]
name = "pandas"
version = "0.1.0"

[build-system]
requires = ["setuptools>=42"]
build-backend = "setuptools.build_meta"
```

By default, uv uses `tool.uv.sources` when resolving build dependencies. For example, this
configuration builds with a local version of `setuptools`:

```toml title="pyproject.toml"
[project]
name = "pandas"
version = "0.1.0"

[build-system]
requires = ["setuptools>=42"]
build-backend = "setuptools.build_meta"

[tool.uv.sources]
setuptools = { path = "./packages/setuptools" }
```

Before publication, `uv build --no-sources` verifies that a package builds without
`tool.uv.sources`. Other build tools, such as [`pypa/build`](https://github.com/pypa/build), do not
use this table.

## Editable dependencies

A regular installation builds a wheel and copies its source files into the virtual environment.
Later changes to the original source files do not update the installed copies.

An editable installation adds a `.pth` file that links the virtual environment to the project. The
interpreter then uses the project's source files directly.

Editable installations require build backend support. They also do not rebuild native modules before
import. However, they are useful during development because the environment always uses the current
package source.

By default, uv installs workspace packages in editable mode.

The `--editable` option adds an editable dependency:

```console
$ uv add --editable ./path/foo
```

The `--no-editable` option disables editable installation for a workspace dependency:

```console
$ uv add --no-editable ./path/foo
```

## Virtual dependencies

A "virtual" dependency is not installed as a [package](./config.md#project-packaging). Its own
dependencies are still installed.

By default, dependencies are not virtual.

A [`path` dependency](#path) can be virtual when its project explicitly sets
[`tool.uv.package = false`](../../reference/settings.md#package). Without that setting, uv treats
the dependency as a normal package and attempts to build it. This applies even if the dependency
does not declare a [build system](./config.md#build-systems).

The `package = false` source setting also makes a dependency virtual:

```toml title="pyproject.toml"
[project]
dependencies = ["bar"]

[tool.uv.sources]
bar = { path = "../projects/bar", package = false }
```

The `package = true` source setting overrides `tool.uv.package = false` in a dependency:

```toml title="pyproject.toml"
[project]
dependencies = ["bar"]

[tool.uv.sources]
bar = { path = "../projects/bar", package = true }
```

A [`workspace` dependency](#workspace-member) can also be virtual when it explicitly sets
[`tool.uv.package = false`](../../reference/settings.md#package). Without that setting, uv builds
the workspace member even if it does not declare a [build system](./config.md#build-systems).

Workspace members that are _not_ dependencies can be virtual by default. For example, a parent
project can use this configuration:

```toml title="pyproject.toml"
[project]
name = "parent"
version = "1.0.0"
dependencies = []

[tool.uv.workspace]
members = ["child"]
```

The child project can omit a build system:

```toml title="pyproject.toml"
[project]
name = "child"
version = "1.0.0"
dependencies = ["anyio"]
```

uv installs the transitive dependency `anyio` but does not install the `child` workspace member.

In contrast, the parent can declare a dependency on `child`:

```toml title="pyproject.toml"
[project]
name = "parent"
version = "1.0.0"
dependencies = ["child"]

[tool.uv.sources]
child = { workspace = true }

[tool.uv.workspace]
members = ["child"]
```

uv then builds and installs `child`.

## Dependency specifiers

uv uses standard
[dependency specifiers](https://packaging.python.org/en/latest/specifications/dependency-specifiers/)
defined by [PEP 508](https://peps.python.org/pep-0508/). A dependency specifier contains these parts
in order:

- The dependency name
- The requested extras (optional)
- The version specifier
- An environment marker (optional)

Comma-separated version specifiers combine multiple constraints. For example,
`foo >=1.2.3,<2,!=1.4.0` selects a `foo` version that is at least 1.2.3, less than 2, and not 1.4.0.

uv adds trailing zeros to specifiers when needed, so `foo ==2` also matches `foo` 2.0.0.

An asterisk can replace the final digit in an equality constraint. For example, `foo ==2.1.*`
accepts any release in the 2.1 series. The `~=` operator accepts compatible versions where the last
digit is equal or higher. `foo ~=1.2` is equivalent to `foo >=1.2,<2`. `foo ~=1.2.3` is equivalent
to `foo >=1.2.3,<1.3`.

Extras appear between the package name and version in comma-separated square brackets. For example:
`pandas[excel,plot] ==2.2`. uv ignores spaces between extra names.

Some dependencies apply only to specific Python versions or operating systems. For example,
`importlib-metadata >=7.1.0,<8; python_version < '3.10'` installs the `importlib.metadata` backport
only on older Python versions. `colorama >=0.4.6,<5; platform_system == "Windows"` installs
`colorama` only on Windows.

Markers can be combined with `and`, `or`, and parentheses. For example:
`aiohttp >=3.7.4,<4; (sys_platform != 'win32' or implementation_name != 'pypy') and python_version >= '3.10'`.
Versions inside markers must be quoted. Versions _outside_ markers must _not_ be quoted.
