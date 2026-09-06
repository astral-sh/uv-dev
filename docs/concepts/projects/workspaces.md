# Using workspaces

A workspace manages one or more packages, called _workspace members_, together. The concept comes
from [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html).

Workspaces organize large codebases into separate packages with shared dependencies. For example,
one Git repository can contain a FastAPI application and several Python libraries. Each library can
have its own version.

Each workspace package has its own `pyproject.toml`. All packages share one lockfile, so the
workspace uses a consistent set of dependencies.

`uv lock` operates on the entire workspace. By default, `uv run` and `uv sync` operate on the
workspace root. Both commands accept `--package` to select a specific workspace member from any
workspace directory.

## Getting started

A `tool.uv.workspace` table in `pyproject.toml` creates a workspace. The package that contains this
table is the workspace root.

!!! tip

    By default, `uv init` inside an existing package adds the new package to the workspace. If the
    workspace root does not have a `tool.uv.workspace` table, uv creates one.

The `members` key is required and lists glob patterns for directories to include. The optional
`exclude` key lists glob patterns for directories to exclude:

```toml title="pyproject.toml"
[project]
name = "albatross"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["bird-feeder", "tqdm>=4,<5"]

[tool.uv.sources]
bird-feeder = { workspace = true }

[tool.uv.workspace]
members = ["packages/*"]
exclude = ["packages/seeds"]
```

Each included directory that is not excluded must contain a `pyproject.toml` file. Workspace members
can be [applications](./init.md#applications) or [libraries](./init.md#libraries).

Every workspace has a root, which is also a workspace member. In this example, `albatross` is the
root. All projects in the `packages` directory are members except `seeds`.

By default, `uv run` and `uv sync` operate on the workspace root. Here, `uv run` and
`uv run --package albatross` are equivalent. `uv run --package bird-feeder` runs the command in the
`bird-feeder` package.

## Workspace sources

The [`tool.uv.sources`](./dependencies.md) table defines dependencies on workspace members:

```toml title="pyproject.toml"
[project]
name = "albatross"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["bird-feeder", "tqdm>=4,<5"]

[tool.uv.sources]
bird-feeder = { workspace = true }

[tool.uv.workspace]
members = ["packages/*"]

[build-system]
requires = ["uv_build>=0.12.10,<0.13"]
build-backend = "uv_build"
```

In this example, the `albatross` project depends on the `bird-feeder` workspace member. The
`workspace = true` entry tells uv to use the workspace package instead of a package from PyPI or
another registry.

The `workspace` field also accepts a path to another workspace. uv resolves the path relative to the
project that declares the source. For a workspace-level source, uv resolves the path relative to the
workspace root. The path must point to the other workspace root. uv selects the member with the same
name as the dependency.

!!! note

    Dependencies between workspace members are editable.

Source definitions in the workspace root apply to all members unless a member overrides them in its
own `tool.uv.sources` table. For example:

```toml title="pyproject.toml"
[project]
name = "albatross"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["bird-feeder", "tqdm>=4,<5"]

[tool.uv.sources]
bird-feeder = { workspace = true }
tqdm = { git = "https://github.com/tqdm/tqdm" }

[tool.uv.workspace]
members = ["packages/*"]

[build-system]
requires = ["uv_build>=0.12.10,<0.13"]
build-backend = "uv_build"
```

By default, every workspace member installs `tqdm` from GitHub. A member can override the `tqdm`
entry in its own `tool.uv.sources` table.

!!! note

    If a member defines a source for a dependency, uv ignores the workspace root's source for that
    dependency. This also applies when the member's source has a
    [marker](dependencies.md#platform-specific-sources) that does not match the current platform.

## Workspace layouts

The most common workspace layout contains a root project and related libraries.

In this example, `albatross` is the root. The `packages` directory contains two libraries,
`bird-feeder` and `seeds`:

```text
albatross
├── packages
│   ├── bird-feeder
│   │   ├── pyproject.toml
│   │   └── src
│   │       └── bird_feeder
│   │           ├── __init__.py
│   │           └── foo.py
│   └── seeds
│       ├── pyproject.toml
│       └── src
│           └── seeds
│               ├── __init__.py
│               └── bar.py
├── pyproject.toml
├── README.md
├── uv.lock
└── src
    └── albatross
        └── __init__.py
```

Because `pyproject.toml` excludes `seeds`, the workspace has two members: `albatross`, the root, and
`bird-feeder`.

## When (not) to use workspaces

Workspaces support the development of related packages in one repository. A large codebase can be
split into smaller packages. Each package can have its own dependencies and version constraints.

Workspaces keep package responsibilities separate. For example, uv has separate packages for its
core library and command-line interface. This makes it possible to test each package independently.

Other common use cases for workspaces include:

- A library with a performance-critical extension module written in Rust, C++, or another language.
- A library with a plugin system where each plugin is a workspace package that depends on the root.

Workspaces are _not_ suitable when members have conflicting requirements or need separate virtual
environments. Path dependencies are often a better choice. For example, each package can be a
separate project instead of a member of the `albatross` workspace. The `tool.uv.sources` table can
define dependencies between those projects as paths:

```toml title="pyproject.toml"
[project]
name = "albatross"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["bird-feeder", "tqdm>=4,<5"]

[tool.uv.sources]
bird-feeder = { path = "packages/bird-feeder" }

[build-system]
requires = ["uv_build>=0.12.10,<0.13"]
build-backend = "uv_build"
```

This approach provides similar benefits and more control over dependency resolution and virtual
environments. However, `uv run --package` is not available. Commands must run from the relevant
package directory.

Workspaces use one `requires-python` range for all members. That range is the intersection of each
member's `requires-python` value. A member may need testing on a Python version that other members
do not support. In that case, `uv pip` can install the member in a separate virtual environment.

!!! note

    Python does not isolate dependencies, so uv cannot prevent a package from importing undeclared
    dependencies. In a workspace, a package can import dependencies declared by another member.
