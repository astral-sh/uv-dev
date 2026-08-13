# Configuring projects

## Python version requirement

The `project.requires-python` field in `pyproject.toml` declares the Python versions that a project
supports.

A `requires-python` value is recommended:

```toml title="pyproject.toml" hl_lines="4"
[project]
name = "example"
version = "0.1.0"
requires-python = ">=3.12"
```

The Python version requirement determines which Python syntax the project can use. It also affects
dependency selection because dependencies must support the same Python version range.

## Entry points

[Entry points](https://packaging.python.org/en/latest/specifications/entry-points/#entry-points)
declare the interfaces that an installed package provides. These include:

- [Command line interfaces](#command-line-interfaces)
- [Graphical user interfaces](#graphical-user-interfaces)
- [Plugin entry points](#plugin-entry-points)

!!! important

    Entry point tables require a [build system](#build-systems).

### Command-line interfaces

The `[project.scripts]` table in `pyproject.toml` defines command-line interfaces (CLIs).

For example, to declare a command called `hello` that invokes the `hello` function in the `example`
module:

```toml title="pyproject.toml"
[project.scripts]
hello = "example:hello"
```

The command then runs from a console:

```console
$ uv run hello
```

### Graphical user interfaces

The `[project.gui-scripts]` table in `pyproject.toml` defines graphical user interfaces (GUIs).

!!! important

    These differ from [command-line interfaces](#command-line-interfaces) only on Windows. Windows
    wraps them in a GUI executable that starts without a console. On other platforms, both types
    behave the same way.

For example, to declare a command called `hello` that invokes the `app` function in the `example`
module:

```toml title="pyproject.toml"
[project.gui-scripts]
hello = "example:app"
```

### Plugin entry points

Projects can define plugin discovery entry points in the
[`[project.entry-points]`](https://packaging.python.org/en/latest/guides/creating-and-discovering-plugins/#using-package-metadata)
table of the `pyproject.toml`.

For example, to register the `example-plugin-a` package as a plugin for `example`:

```toml title="pyproject.toml"
[project.entry-points.'example.plugins']
a = "example_plugin_a"
```

The `example` package can load the plugins with:

```python title="example/__init__.py"
from importlib.metadata import entry_points

for plugin in entry_points(group="example.plugins"):
    plugin.load()
```

!!! note

    The `group` key accepts any value. It does not need to contain the package name or "plugins".
    Including the package name is recommended because it avoids conflicts with other packages.

## Build systems

A build system determines how to package and install a project. The `[build-system]` table in
`pyproject.toml` declares and configures the build system.

uv uses the build system to determine whether a project contains a package to install in its virtual
environment. If the project has no build system, uv installs its dependencies but does not build or
install the project. If the project has a build system, uv builds and installs the project.

By default, `uv init` creates a packaged project using the [uv build backend](../build-backend.md).
The `--build-backend` option selects another build backend. The `--no-package` option creates a
flat, unpackaged project instead.

!!! note

    uv does not build or install the current project without a build system. Other packages do not
    require a `[build-system]` table. For compatibility, uv builds packages without a declared build
    system with `setuptools.build_meta:__legacy__`. Dependencies can therefore be installed without
    an explicit build system. uv also attempts to build and install
    [local project dependencies](./dependencies.md#path) and packages installed with `uv pip`, even
    when they do not declare a build system.

Build systems support these features:

- Including or excluding files from distributions
- Editable installation behavior
- Dynamic project metadata
- Compilation of native code
- Vendoring shared libraries

The documentation for each build system describes how to configure these features.

## Project packaging

As described in [build systems](#build-systems), a Python project must be built before installation.
This process is called "packaging".

A project usually needs a package to:

- Add commands to the project
- Distribute the project to others
- Use a `src` and `test` layout
- Write a library

A project usually _does not_ need a package for:

- Writing scripts
- Building a simple application
- Using a flat layout

uv usually determines whether to package a project from its [build system](#build-systems). The
[`tool.uv.package`](../../reference/settings.md#package) setting overrides this behavior.

Setting `tool.uv.package = true` forces uv to build and install the project. If the project has no
build system, uv uses the setuptools legacy backend.

Setting `tool.uv.package = false` prevents uv from building and installing the project package. uv
ignores any declared build system during project operations. Explicit build commands such as
`uv build` still build the project.

## Project environment path

The `UV_PROJECT_ENVIRONMENT` environment variable configures the path to the project virtual
environment. The default path is `.venv`.

uv resolves relative paths from the workspace root. It uses absolute paths directly and does not
create a child directory for the environment. If the specified environment does not exist, uv
creates it.

This option can target the system Python environment, but this is not recommended. By default,
`uv sync` removes packages that the project does not require. This can break the system environment.

The system environment is selected by setting `UV_PROJECT_ENVIRONMENT` to the Python installation
prefix. On Debian-based systems, that prefix is usually `/usr/local`:

```console
$ python -c "import sysconfig; print(sysconfig.get_config_var('prefix'))"
/usr/local
```

The setting `UV_PROJECT_ENVIRONMENT=/usr/local` selects this environment.

!!! important

    If multiple projects use the same absolute path, each project overwrites the environment. An
    absolute path is recommended only for one project in CI or a Docker image.

!!! note

    By default, uv does not read `VIRTUAL_ENV` during project operations. If `VIRTUAL_ENV` points to
    a different environment, uv displays a warning. The `--active` option makes uv use
    `VIRTUAL_ENV`. The `--no-active` option hides the warning.

## Build isolation

By default, uv builds packages in isolated virtual environments with their declared build
dependencies. This follows [PEP 517](https://peps.python.org/pep-0517/).

Some packages do not support this form of build isolation. For example,
[`flash-attn`](https://pypi.org/project/flash-attn/) and
[`deepspeed`](https://pypi.org/project/deepspeed/) must build against the PyTorch version in the
project environment. An isolated build can select a different PyTorch version and cause runtime
errors.

Other packages do not declare all their build dependencies. For example,
[`cchardet`](https://pypi.org/project/cchardet/) requires `cython` before installation but does not
declare `cython` as a build dependency.

uv supports two ways to change build isolation:

1. **Augmenting the list of build dependencies**: The
   [`extra-build-dependencies`](../../reference/settings.md#extra-build-dependencies) setting adds
   undeclared build dependencies to an isolated environment. A build dependency such as `torch` can
   also be matched to the version in the project environment.

1. **Disabling build isolation for specific packages**: uv can build selected packages in the
   project environment instead of an isolated environment.

Additional build dependencies are preferred when possible. Without build isolation, build
dependencies must already be installed in the project environment. This can make installation more
complex, leave extra packages in the environment, and make the environment harder to reproduce.

### Augmenting build dependencies

The [`extra-build-dependencies`](../../reference/settings.md#extra-build-dependencies) table in
`pyproject.toml` specifies additional build dependencies for each package.

For example, the following configuration builds `cchardet` with `cython` as an additional build
dependency:

```toml title="pyproject.toml"
[project]
name = "project"
version = "0.1.0"
description = "..."
readme = "README.md"
requires-python = ">=3.12"
dependencies = ["cchardet"]

[tool.uv.extra-build-dependencies]
cchardet = ["cython"]
```

The `match-runtime = true` setting selects the same build dependency version as the project
environment. For example, the following configuration builds `deepspeed` with `torch`:

```toml title="pyproject.toml"
[project]
name = "project"
version = "0.1.0"
description = "..."
readme = "README.md"
requires-python = ">=3.12"
dependencies = ["deepspeed", "torch"]

[tool.uv.extra-build-dependencies]
deepspeed = [{ requirement = "torch", match-runtime = true }]
```

This builds `deepspeed` with the same `torch` version that the project environment uses.

!!! tip

    Pre-built `deepspeed` wheels are also available from the
    [Astral GPU indexes](../../guides/integration/pytorch.md#installing-gpu-enabled-pytorch-extensions).

The same approach can build `flash-attn` with `torch` as an additional build dependency:

```toml title="pyproject.toml"
[project]
name = "project"
version = "0.1.0"
description = "..."
readme = "README.md"
requires-python = ">=3.12"
dependencies = ["flash-attn", "torch"]

[tool.uv.extra-build-dependencies]
flash-attn = [{ requirement = "torch", match-runtime = true }]

[tool.uv.extra-build-variables]
flash-attn = { FLASH_ATTENTION_SKIP_CUDA_BUILD = "TRUE" }
```

!!! note

    The `FLASH_ATTENTION_SKIP_CUDA_BUILD` variable lets uv resolve `flash-attn` from a pre-built
    wheel. A source build requires the CUDA development toolkit.

    If the CUDA toolkit is available during resolution, omitting
    `FLASH_ATTENTION_SKIP_CUDA_BUILD` is recommended. Setting it to `TRUE` can produce an
    incompatible installation if no wheel supports the target PyTorch version, GPU, and platform.

!!! tip

    Pre-built `flash-attn` wheels are also available from the
    [Astral GPU indexes](../../guides/integration/pytorch.md#installing-gpu-enabled-pytorch-extensions).

[`deep_gemm`](https://github.com/deepseek-ai/DeepGEMM) follows the same pattern:

```toml title="pyproject.toml"
[project]
name = "project"
version = "0.1.0"
description = "..."
readme = "README.md"
requires-python = ">=3.12"
dependencies = ["deep_gemm", "torch"]

[tool.uv.sources]
deep_gemm = { git = "https://github.com/deepseek-ai/DeepGEMM" }

[tool.uv.extra-build-dependencies]
deep_gemm = [{ requirement = "torch", match-runtime = true }]
```

!!! tip

    Pre-built `deep_gemm` wheels are also available from the
    [Astral GPU indexes](../../guides/integration/pytorch.md#installing-gpu-enabled-pytorch-extensions).

The uv cache tracks `extra-build-dependencies` and `extra-build-variables`. Changing either setting
rebuilds and reinstalls the affected packages. For example, upgrading `torch` rebuilds `flash-attn`
with the new `torch` version.

#### Dynamic metadata

The `match-runtime = true` setting requires static package metadata. Without static metadata, uv
must build the package during dependency resolution. At that point, uv does not yet know which
version of the build dependency the project environment will use.

For example, without static `flash-attn` metadata, uv would need to build `flash-attn` before it
could resolve the `torch` version.

[`axolotl`](https://pypi.org/project/axolotl/) needs additional build dependencies but does not
declare static metadata. Its dependencies depend on the installed `torch` version. The project must
therefore specify an exact `torch` version and add that version as a build dependency.

For example, this configuration builds `axolotl` with `torch==2.6.0`:

```toml title="pyproject.toml"
[project]
name = "project"
version = "0.1.0"
description = "..."
readme = "README.md"
requires-python = ">=3.12"
dependencies = ["axolotl[deepspeed, flash-attn]", "torch==2.6.0"]

[tool.uv.extra-build-dependencies]
axolotl = ["torch==2.6.0"]
deepspeed = ["torch==2.6.0"]
flash-attn = ["torch==2.6.0"]
```

Older versions of `flash-attn` also lacked static metadata and did not directly support
`match-runtime = true`. Unlike `axolotl`, their dependencies did not change with the build
environment. The [`dependency-metadata`](../../reference/settings.md#dependency-metadata) setting
can provide their metadata in advance. This avoids building the package during dependency
resolution. For example:

```toml title="pyproject.toml"
[[tool.uv.dependency-metadata]]
name = "flash-attn"
version = "2.6.3"
requires-dist = ["torch", "einops"]
```

!!! tip

    Package metadata is available from the package's Git repository or its source distribution on
    [PyPI](https://pypi.org/project/flash-attn). Package requirements are usually in `setup.py` or
    `setup.cfg`.

    A built distribution contains a `METADATA` file. If a built distribution is available, uv can
    already read its metadata, so providing the metadata separately is unnecessary.

    The `version` field in `tool.uv.dependency-metadata` is optional for registry dependencies. If
    omitted, the metadata applies to all package versions. The field is _required_ for direct URL
    dependencies, including Git dependencies.

### Disabling build isolation

Without build isolation, a package's build dependencies must be installed in the project environment
_before_ the package is built.

For example, installing `cchardet` without build isolation previously required separate commands.
First, `cython` and `setuptools` were installed. Then, `cchardet` was installed without isolation:

```console
$ uv venv
$ uv pip install cython setuptools
$ uv pip install cchardet --no-build-isolation
```

The `no-build-isolation-package` setting in `pyproject.toml` disables isolation for selected
packages. The `--no-build-isolation-package` command-line option has the same effect.

uv first installs packages that support isolated builds. It then installs packages that do not. When
build dependencies are also project dependencies, uv installs them before the package that requires
them.

For example, this configuration installs `cchardet` without build isolation:

```toml title="pyproject.toml"
[project]
name = "project"
version = "0.1.0"
description = "..."
readme = "README.md"
requires-python = ">=3.12"
dependencies = ["cchardet", "cython", "setuptools"]

[tool.uv]
no-build-isolation-package = ["cchardet"]
```

`uv sync` first installs `cython` and `setuptools`. It then installs `cchardet` without build
isolation:

```console
$ uv sync --extra build
 + cchardet==2.1.7
 + cython==3.1.3
 + setuptools==80.9.0
```

The same approach can install `flash-attn` without build isolation:

```toml title="pyproject.toml"
[project]
name = "project"
version = "0.1.0"
description = "..."
readme = "README.md"
requires-python = ">=3.12"
dependencies = ["flash-attn", "torch"]

[tool.uv]
no-build-isolation-package = ["flash-attn"]
```

`uv sync` first installs `torch`. It then installs `flash-attn` without build isolation. Because
`torch` is both a project dependency and a build dependency, both environments use the same version.

This approach keeps build dependencies in the project environment. That works for `flash-attn`,
which needs `torch` during both builds and runtime. It is less suitable for `cchardet`, which needs
`cython` only during builds.

A two-step installation can keep build dependencies separate from the packages that require them.
For example, the `cchardet` build dependencies can be placed in an optional `build` group:

```toml title="pyproject.toml"
[project]
name = "project"
version = "0.1.0"
description = "..."
readme = "README.md"
requires-python = ">=3.12"
dependencies = ["cchardet"]

[project.optional-dependencies]
build = ["setuptools", "cython"]

[tool.uv]
no-build-isolation-package = ["cchardet"]
```

The first sync includes the optional `build` group. The second sync excludes it and removes the
build dependencies:

```console
$ uv sync --extra build
 + cchardet==2.1.7
 + cython==3.1.3
 + setuptools==80.9.0
$ uv sync
 - cython==3.1.3
 - setuptools==80.9.0
```

Some packages, such as `cchardet`, need build dependencies only during the _installation_ phase of
`uv sync`. Other packages also need them during dependency _resolution_.

For those packages, the lower-level `uv pip` interface can install build dependencies before
`uv lock` or `uv sync` runs. For example:

```toml title="pyproject.toml"
[project]
name = "project"
version = "0.1.0"
description = "..."
readme = "README.md"
requires-python = ">=3.12"
dependencies = ["flash-attn"]

[tool.uv]
no-build-isolation-package = ["flash-attn"]
```

The following commands sync `flash-attn`:

```console
$ uv venv
$ uv pip install torch setuptools
$ uv sync
```

Alternatively, the [`dependency-metadata`](../../reference/settings.md#dependency-metadata) setting
can provide `flash-attn` metadata in advance. This avoids building the package during dependency
resolution. For example:

```toml title="pyproject.toml"
[[tool.uv.dependency-metadata]]
name = "flash-attn"
version = "2.6.3"
requires-dist = ["torch", "einops"]
```

## Editable mode

By default, uv installs the project in editable mode. Changes to the source code are immediately
available in the environment. Both `uv sync` and `uv run` accept `--no-editable` to install the
project in non-editable mode. This option supports deployments such as Docker containers, where the
installed project must not depend on its original source directory.

## Conflicting dependencies

uv resolves all project dependencies together, including optional dependencies ("extras") and
dependency groups. If dependencies from different sections are incompatible, resolution fails.

uv supports explicit declarations of conflicting dependency groups. For example, the following
configuration declares that the `optional-dependency` groups `extra1` and `extra2` are incompatible:

```toml title="pyproject.toml"
[tool.uv]
conflicts = [
    [
      { extra = "extra1" },
      { extra = "extra2" },
    ],
]
```

The following configuration declares that the development dependency groups `group1` and `group2`
are incompatible:

```toml title="pyproject.toml"
[tool.uv]
conflicts = [
    [
      { group = "group1" },
      { group = "group2" },
    ],
]
```

The [resolution documentation](../resolution.md#conflicting-dependencies) provides more details.

## Limited resolution environments

The `environments` setting limits resolution to specific platforms or Python versions. It accepts a
list of PEP 508 environment markers. For example, the following configuration limits the lockfile to
macOS and Linux and excludes Windows:

```toml title="pyproject.toml"
[tool.uv]
environments = [
    "sys_platform == 'darwin'",
    "sys_platform == 'linux'",
]
```

The [resolution documentation](../resolution.md#limited-resolution-environments) provides more
details.

## Required environments

The `required-environments` setting marks specific platforms or Python versions as required. For
example, the following configuration requires support for Intel macOS:

```toml title="pyproject.toml"
[tool.uv]
required-environments = [
    "sys_platform == 'darwin' and platform_machine == 'x86_64'",
]
```

The `required-environments` setting affects packages without source distributions, such as PyTorch.
These packages can _only_ be installed in environments that match a published pre-built wheel.

The [resolution documentation](../resolution.md#required-environments) provides more details.
