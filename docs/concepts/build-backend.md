# The uv build backend

A build backend converts a source directory into a source distribution or a wheel.

uv supports all build backends that follow [PEP 517](https://peps.python.org/pep-0517/). It also
provides a native build backend, `uv_build`, that integrates with uv to improve performance and
usability.

## Choosing a build backend

The uv build backend works well for most Python projects. Its defaults require no configuration for
most projects. Configuration options support other common project structures. The backend integrates
with uv to provide clear messages and fast builds. It also validates project metadata and structure
to prevent common mistakes.

The uv build backend **supports pure Python code only**. A
[library with extension modules](../concepts/projects/init.md#projects-with-extension-modules)
requires another backend.

!!! tip

    If a project requires build scripts or a more flexible layout, consider the
    [hatchling](https://hatch.pypa.io/latest/config/build/#build-system) build backend instead.

## Using the uv build backend

To use uv as a build backend in an existing project, add `uv_build` to the
[`[build-system]`](../concepts/projects/config.md#build-systems) section in `pyproject.toml`:

```toml title="pyproject.toml"
[build-system]
requires = ["uv_build>=0.12.10,<0.13"]
build-backend = "uv_build"
```

!!! note

    The uv build backend follows the same [versioning policy](../reference/policies/versioning.md)
    as uv. Include an upper bound on the `uv_build` version to keep builds compatible with future
    releases.

To create a new project that uses the uv build backend, use `uv init`:

```console
$ uv init
```

Commands such as [`uv build`](../guides/package.md) then use this backend to create the source
distribution and wheel.

## Bundled build backend

The `uv_build` package contains a portable build backend with a small binary. The `uv` executable
also includes a copy of this backend. During commands such as `uv build`, uv uses the bundled copy
if its version satisfies the `uv_build` requirement. Otherwise, uv uses a compatible version of the
`uv_build` package. Other build frontends, such as `python -m build`, always use the `uv_build`
package. They usually select its latest compatible version.

## Modules

The uv build backend expects Python packages to contain one or more modules. By default, it expects
one root package module with an `__init__.py` file at `src/<package_name>/__init__.py`.

For example, a project named `foo` has this structure:

```text
pyproject.toml
src
└── foo
    └── __init__.py
```

uv normalizes the package name to determine the default module name. It converts the name to
lowercase and replaces dots and dashes with underscores. For example, `Foo-Bar` becomes `foo_bar`.

The `src/` directory is the default directory for module discovery.

Use the `module-name` and `module-root` settings to change these defaults. For example, a `FOO`
module in the root directory has this structure:

```text
pyproject.toml
FOO
└── __init__.py
```

Use this build configuration:

```toml title="pyproject.toml"
[tool.uv.build-backend]
module-name = "FOO"
module-root = ""
```

## Namespace packages

Namespace packages let multiple packages place modules in a shared namespace.

A `.` in `module-name` identifies a namespace package module. For example, use this structure to
package the `bar` module in the shared `foo` namespace:

```text
pyproject.toml
src
└── foo
    └── bar
        └── __init__.py
```

Set `module-name` as follows:

```toml title="pyproject.toml"
[tool.uv.build-backend]
module-name = "foo.bar"
```

!!! important

    Do not add `__init__.py` to `foo` because `foo` is the shared namespace module.

A namespace package can also contain more than one root module:

```text
pyproject.toml
src
├── foo
│   └── __init__.py
└── bar
    └── __init__.py
```

Use a workspace with multiple packages when possible. If this structure is necessary, set
`module-name` to a list of names:

```toml title="pyproject.toml"
[tool.uv.build-backend]
module-name = ["foo", "bar"]
```

For packages with many modules or complex namespaces, set `namespace = true` to avoid listing every
module name:

```toml title="pyproject.toml"
[tool.uv.build-backend]
namespace = true
```

!!! warning

    `namespace = true` disables safety checks. Use an explicit list of module names unless the
    project requires this legacy behavior.

Combine `namespace` with `module-name` to declare the root explicitly. For example, consider this
project structure:

```text
pyproject.toml
src
└── foo
    ├── bar
    │   └── __init__.py
    └── baz
        └── __init__.py
```

Use this configuration:

```toml title="pyproject.toml"
[tool.uv.build-backend]
module-name = "foo"
namespace = true
```

## Stub packages

The build backend also builds type stub packages. These packages have a `-stubs` suffix in the
package or module name, such as `foo-stubs`. Because type stub module names must end in `-stubs`, uv
does not replace the `-` with an underscore. uv also searches for an `__init__.pyi` file. For
example, use this project structure:

```text
pyproject.toml
src
└── foo-stubs
    └── __init__.pyi
```

uv also supports type stub modules in [namespace packages](#namespace-packages).

## File inclusion and exclusion

The build backend determines which source files to package in distributions.

To build a source distribution, uv first adds included files and directories. It then removes
excluded files and directories. Exclusions therefore take precedence over inclusions.

By default, uv excludes `__pycache__`, `*.pyc`, and `*.pyo`.

uv includes these files and directories in source distributions:

- The `pyproject.toml` file. If uv detects TOML 1.1-only syntax, it warns and enables the
  `toml-backwards-compatibility` preview feature. uv reformats `pyproject.toml` for backwards
  compatibility and saves the original as `pyproject.toml.orig`. Pass
  `--preview-feature toml-backwards-compatibility` to enable this feature explicitly and suppress
  the warning.
- The [module](#modules) under
  [`tool.uv.build-backend.module-root`](../reference/settings.md#build-backend_module-root).
- The files referenced by `project.license-files` and `project.readme`.
- All directories under [`tool.uv.build-backend.data`](../reference/settings.md#build-backend_data).
- All files matching patterns from
  [`tool.uv.build-backend.source-include`](../reference/settings.md#build-backend_source-include).

uv then removes files that match
[`tool.uv.build-backend.source-exclude`](../reference/settings.md#build-backend_source-exclude) or
the [default excludes](../reference/settings.md#build-backend_default-excludes).

uv includes these files and directories in wheels:

- The [module](#modules) under
  [`tool.uv.build-backend.module-root`](../reference/settings.md#build-backend_module-root)
- The files referenced by `project.license-files`, which are copied into the `.dist-info` directory.
- The `project.readme`, which is copied into the project metadata.
- All directories under [`tool.uv.build-backend.data`](../reference/settings.md#build-backend_data),
  which are copied into the `.data` directory.

uv then removes files that match
[`tool.uv.build-backend.source-exclude`](../reference/settings.md#build-backend_source-exclude),
[`tool.uv.build-backend.wheel-exclude`](../reference/settings.md#build-backend_wheel-exclude), or
the default excludes. Source exclusions keep direct wheel builds consistent with wheels built from
source distributions.

The uv build backend does not support separate wheel include settings. By default, it includes one
top-level module. Additional modules require explicit configuration. Data files must appear under
the module root or in the appropriate [data directory](../reference/settings.md#build-backend_data).
Most packages store small data files beside the source code in the module root.

!!! tip

    For other build frontends, such as pip or `python -m build`, set `RUST_LOG=uv=debug` or
    `RUST_LOG=uv=verbose` to enable debug logging. When uv invokes the backend, the backend uses uv's
    verbosity level.

### Include and exclude syntax

Include patterns are anchored to the project root. For example, `pyproject.toml` includes
`<root>/pyproject.toml`, but not `<root>/bar/pyproject.toml`. Add `/**` to include every file in a
directory and its subdirectories, such as `src/**`. Recursive patterns are also anchored. For
example, `assets/**/sample.csv` includes every `sample.csv` file in `<root>/assets` and its
subdirectories.

!!! note

    For performance and reproducibility, avoid patterns without an anchor such as `**/sample.csv`.

Exclude patterns are not anchored. For example, `__pycache__` excludes every directory with that
name, regardless of its parent. uv also excludes all files and subdirectories in an excluded
directory. Add a `/` prefix to anchor a pattern. For example, `/dist` excludes only `<root>/dist`.

All pattern fields use the reduced portable glob syntax from
[PEP 639](https://peps.python.org/pep-0639/#add-license-FILES-key). A backslash also escapes special
characters.
