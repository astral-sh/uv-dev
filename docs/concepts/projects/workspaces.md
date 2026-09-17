# Using workspaces

Inspired by the [Cargo](https://doc.rust-lang.org/cargo/reference/workspaces.html) concept of the
same name, a workspace is "a collection of one or more packages, called _workspace members_, that
are managed together."

Workspaces organize large codebases by splitting them into multiple packages with common
dependencies. Think: a FastAPI-based web application, alongside a series of libraries that are
versioned and maintained as separate Python packages, all in the same Git repository.

In a workspace, each package defines its own `pyproject.toml`, but the workspace shares a single
lockfile. By default, all members must have compatible requirements. Workspaces with intentional
differences can opt into [resolution axes](#resolution-axes) while keeping a shared lockfile.

As such, `uv lock` operates on the entire workspace at once, while `uv run` and `uv sync` operate on
the workspace root by default, though both accept a `--package` argument, allowing you to run a
command in a particular workspace member from any workspace directory.

## Getting started

To create a workspace, add a `tool.uv.workspace` table to a `pyproject.toml`, which will implicitly
create a workspace rooted at that package.

!!! tip

    By default, running `uv init` inside an existing package will add the newly created member to the workspace, creating a `tool.uv.workspace` table in the workspace root if it doesn't already exist.

In defining a workspace, you must specify the `members` (required) and `exclude` (optional) keys,
which direct the workspace to include or exclude specific directories as members respectively, and
accept lists of globs:

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

Every directory included by the `members` globs (and not excluded by the `exclude` globs) must
contain a `pyproject.toml` file. However, workspace members can be _either_
[applications](./init.md#applications) or [libraries](./init.md#libraries); both are supported in
the workspace context.

Every workspace needs a root, which is _also_ a workspace member. In the above example, `albatross`
is the workspace root, and the workspace members include all projects under the `packages`
directory, except `seeds`.

By default, `uv run` and `uv sync` operates on the workspace root. For example, in the above
example, `uv run` and `uv run --package albatross` would be equivalent, while
`uv run --package bird-feeder` would run the command in the `bird-feeder` package.

## Workspace sources

Within a workspace, dependencies on workspace members are facilitated via
[`tool.uv.sources`](./dependencies.md), as in:

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
requires = ["uv_build>=0.12.15,<0.13"]
build-backend = "uv_build"
```

In this example, the `albatross` project depends on the `bird-feeder` project, which is a member of
the workspace. The `workspace = true` key-value pair in the `tool.uv.sources` table indicates the
`bird-feeder` dependency should be provided by the workspace, rather than fetched from PyPI or
another registry. The `workspace` field can also be set to a path string to resolve a dependency
from a different workspace. The path is resolved relative to the project that declares the source
(or the workspace root for a workspace-level source) and must point to the external workspace root.
uv selects the member that matches the dependency name.

!!! note

    Dependencies between workspace members are editable.

Any `tool.uv.sources` definitions in the workspace root apply to all members, unless overridden in
the `tool.uv.sources` of a specific member. For example, given the following `pyproject.toml`:

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
requires = ["uv_build>=0.12.15,<0.13"]
build-backend = "uv_build"
```

Every workspace member would, by default, install `tqdm` from GitHub, unless a specific member
overrides the `tqdm` entry in its own `tool.uv.sources` table.

!!! note

    If a workspace member provides `tool.uv.sources` for some dependency, it will ignore any
    `tool.uv.sources` for the same dependency in the workspace root, even if the member's source is
    limited by a [marker](dependencies.md#platform-specific-sources) that doesn't match the current
    platform.

## Resolution axes

The [`workspace-resolution-axes` preview feature](../preview.md) supports monorepos whose members
need different Python or dependency versions. An axis defines independently selectable sections,
such as Python 3.12 versus Python 3.13, or SQLAlchemy 1 versus SQLAlchemy 2. An axis can have more
than two sections.

For example, a workspace with `legacy-worker`, `migration-worker`, and `modern-worker` services
could define:

```toml title="pyproject.toml"
[tool.uv]
preview-features = ["workspace-resolution-axes"]

[tool.uv.workspace]
members = ["services/*", "packages/*"]

[tool.uv.workspace.resolution-axes.python]
py312 = { member-paths = ["services/legacy-*"], requires-python = "==3.12.*" }
py313 = { members = ["migration-worker", "modern-worker"], requires-python = "==3.13.*" }

[tool.uv.workspace.resolution-axes.sqlalchemy]
v1 = { members = ["legacy-worker", "migration-worker"], constraint-dependencies = ["sqlalchemy>=1,<2"] }
v2 = { members = ["modern-worker"], constraint-dependencies = ["sqlalchemy>=2,<3"] }

[tool.uv.workspace.resolution-axes.protocol]
v1 = { members = ["legacy-worker"], constraint-dependencies = ["protocol-lib>=1,<2"] }
v2 = { members = ["migration-worker"], constraint-dependencies = ["protocol-lib>=2,<3"] }
v3 = { members = ["modern-worker"], constraint-dependencies = ["protocol-lib>=3,<4"] }
```

The `members` entries are package names from each member's `project.name`. The `member-paths`
entries are workspace-relative globs over members already discovered by `tool.uv.workspace`. They do
not add new members to the workspace.

A member can belong to at most one section of each axis, but it can belong to sections on several
different axes. All of its assignments must match. In this example, `migration-worker` belongs to
`python=py313`, `sqlalchemy=v1`, and `protocol=v2`. A member omitted from an axis is unrestricted on
that axis; a member omitted from every axis is shared by all selections.

A section's `constraint-dependencies` narrows dependencies requested by the selected packages; it
does not add dependencies. A section's `requires-python` narrows the supported Python versions
without changing each project's own `requires-python`. Dependencies on other workspace members must
also be compatible with the selected sections.

### Locking and consistency

`uv lock` records all declared axis combinations in one `uv.lock`. Compatible combinations are
resolved together without enumerating their full product. When a shared resolution is impossible, uv
splits the affected combinations and tries to retain common dependency versions across the resulting
resolutions. These alignment attempts are bounded: uv does not guarantee the globally smallest
number of distinct versions.

`uv tree` and `uv workspace metadata` do not yet support resolution-axis lockfiles. Use `uv export`
with a selection to inspect a concrete resolution.

Normal locking favors applicable versions already recorded for each context in `uv.lock`. Optional
alignment does not replace unaffected existing pins during a selective upgrade. `--upgrade`, or a
change to `--resolution` or `--fork-strategy`, lets uv reconsider those choices and the resolution
partition. The default `requires-python` fork strategy favors recent versions on newer Python
versions; `--fork-strategy fewest` permits sharing an older compatible version across those
environments. `--resolution lowest-direct` retains each context's own direct dependency set. Without
`resolution-axes`, workspace resolution is unchanged.

### Selecting workspace members

`uv run`, `uv sync`, and `uv export` accept repeated `--resolution-axis AXIS=SECTION` options. Use
`--all-matching-packages` to operate on all members whose assignments match the selection:

```console
$ uv sync --all-matching-packages \
    --resolution-axis python=py313 \
    --resolution-axis sqlalchemy=v2 \
    --resolution-axis protocol=v3
```

Requesting a member with `--package` also selects that member's assigned sections. For example,
these commands infer all three assignments for `modern-worker`:

```console
$ uv run --package modern-worker python -m modern_worker
$ uv export --frozen --package modern-worker
```

An omitted axis remains unresolved; uv does not choose a default section. A selector can be omitted
when the requested members imply its value, or when the remaining choices do not affect the
requested members or their locked dependencies. Otherwise, uv reports the ambiguous axes and asks
for an explicit selection.

Explicit package requests remain strict. `--package legacy-worker --package modern-worker` cannot
silently drop either member, and `--all-packages` still requests every member. Use
`--all-matching-packages` when intentionally selecting only the compatible members. Resolution axes
cannot be combined with `tool.uv.workspace.groups`.

The `batch-export` preview also supports a separate selection for each `[[export]]` entry:

```toml title="exports.toml"
[[export]]
output-file = "requirements-modern.txt"
all-matching-packages = true
resolution-axes = { python = "py313", sqlalchemy = "v2", protocol = "v3" }
```

```console
$ uv export --frozen --batch exports.toml --preview-features batch-export
```

Command-line `--resolution-axis` selections are combined with each entry's `resolution-axes`;
contradictory assignments are errors. With `--batch`, set `all-matching-packages` in each entry
instead of passing the command-line flag.

## Workspace layouts

The most common workspace layout can be thought of as a root project with a series of accompanying
libraries.

For example, continuing with the above example, this workspace has an explicit root at `albatross`,
with two libraries (`bird-feeder` and `seeds`) in the `packages` directory:

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

Since `seeds` was excluded in the `pyproject.toml`, the workspace has two members total: `albatross`
(the root) and `bird-feeder`.

## When (not) to use workspaces

Workspaces are intended to facilitate the development of multiple interconnected packages within a
single repository. As a codebase grows in complexity, it can be helpful to split it into smaller,
composable packages, each with their own dependencies and version constraints.

Workspaces help enforce isolation and separation of concerns. For example, in uv, we have separate
packages for the core library and the command-line interface, enabling us to test the core library
independently of the CLI, and vice versa.

Other common use cases for workspaces include:

- A library with a performance-critical subroutine implemented in an extension module (Rust, C++,
  etc.).
- A library with a plugin system, where each plugin is a separate workspace package with a
  dependency on the root.

By default, workspace members must have compatible requirements. The
[resolution axes preview](#resolution-axes) allows intentional differences while retaining a shared
lockfile. If members need independent lockfiles or separately managed virtual environments, path
dependencies are often preferable. For example, rather than grouping `albatross` and its members in
a workspace, you can define each package as its own independent project, with inter-package
dependencies defined as path dependencies in `tool.uv.sources`:

```toml title="pyproject.toml"
[project]
name = "albatross"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["bird-feeder", "tqdm>=4,<5"]

[tool.uv.sources]
bird-feeder = { path = "packages/bird-feeder" }

[build-system]
requires = ["uv_build>=0.12.15,<0.13"]
build-backend = "uv_build"
```

This approach conveys many of the same benefits, but allows for more fine-grained control over
dependency resolution and virtual environment management (with the downside that `uv run --package`
is no longer available; instead, commands must be run from the relevant package directory).

Finally, ordinary workspaces enforce a single `requires-python` for the entire workspace, taking the
intersection of all members' `requires-python` values. Resolution axes can restrict that
intersection to the members and policies of each selection. For a member that needs a separately
managed environment, you can also use `uv pip` to install it in that environment.

!!! note

    As Python does not provide dependency isolation, uv can't ensure that a package uses its declared dependencies and nothing else. For workspaces specifically, uv can't ensure that packages don't import dependencies declared by another workspace member.
