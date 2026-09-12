# Project structure and files

## The `pyproject.toml`

A [`pyproject.toml`](https://packaging.python.org/en/latest/guides/writing-pyproject-toml/) file
defines Python project metadata. uv uses this file to identify the root directory of a project.

!!! tip

    `uv init` creates a new project. [Creating projects](./init.md) describes this command.

A minimal project definition includes a name and version:

```toml title="pyproject.toml"
[project]
name = "example"
version = "0.1.0"
```

Additional project metadata and configuration include:

- [Python version requirement](./config.md#python-version-requirement)
- [Dependencies](./dependencies.md)
- [Build system](./config.md#build-systems)
- [Entry points (commands)](./config.md#entry-points)

## The project environment

uv creates a virtual environment when a project requires one. Some commands create temporary
environments, such as `uv run --isolated`. uv also maintains a persistent project environment in a
`.venv` directory next to `pyproject.toml`. This environment contains the project and its
dependencies.

By default, uv stores `.venv` inside the project directory. Editors can then find the environment
for code completion and type hints. The `.venv` directory should not be included in version control.
An internal `.gitignore` file excludes it from Git automatically.

`uv run` runs a command in the project environment. Standard virtual environment activation also
works with the project environment.

If the project environment does not exist, `uv run` creates it. Otherwise, `uv run` updates the
environment when necessary. `uv sync` also creates the environment explicitly. The
[locking and syncing](./sync.md) documentation describes this behavior.

Direct changes to the project environment, such as `uv pip install`, are _not_ recommended. `uv add`
adds a project dependency to the environment. [`uvx`](../../guides/tools.md) and
[`uv run --with`](./run.md#requesting-additional-dependencies) support one-off requirements.

!!! tip

    The [`managed = false`](../../reference/settings.md#managed) setting disables automatic project
    locking and syncing. For example:

    ```toml title="pyproject.toml"
    [tool.uv]
    managed = false
    ```

### Centralized project environments

With the [`centralized-project-envs` preview feature](../preview.md), uv stores the default project
environment in its cache. uv attempts to link the `.venv` directory to the cached environment.
Existing activation and editor workflows can then continue to use the usual path.

If uv cannot create the link, it attempts to write the cached environment path to `.venv` instead.
If that also fails, uv uses the cached environment directly. Tools that depend on `.venv` might then
fail to find the environment. Changing interpreters selects separate cached environments that uv can
reuse later.

uv does not centralize explicit project environment paths. This includes `UV_PROJECT_ENVIRONMENT`
and environments selected with `--active`. If `--no-cache` is enabled, the feature has no effect.

The feature also applies to `uv venv` commands without a path at a project or workspace root.

## The lockfile

uv creates a `uv.lock` file next to the `pyproject.toml`.

`uv.lock` is a _universal_ or _cross-platform_ lockfile. It records packages across all possible
Python markers, including operating system, architecture, and Python version.

`pyproject.toml` defines broad project requirements. The lockfile records the exact resolved
versions that uv installs in the project environment. The lockfile should be included in version
control. This keeps installations consistent and reproducible across machines.

A lockfile gives project developers a consistent set of package versions. It also records the exact
package versions used when deploying the project as an application.

When commands such as `uv sync` and `uv run` use the project environment, uv
[automatically creates and updates](./sync.md#automatic-lock-and-sync) the lockfile. `uv lock` also
updates the lockfile explicitly.

Although `uv.lock` uses human-readable TOML, uv manages the file. Manual edits are not recommended.
The `uv.lock` format is specific to uv, so other tools cannot use it.

### Relationship to `pylock.toml`

[PEP 751](https://peps.python.org/pep-0751/) standardized a resolution file format named
`pylock.toml`.

The `pylock.toml` resolution output format is intended to replace `requirements.txt`. For example,
`uv pip compile` can generate a locked `requirements.txt` file from input requirements. The
standardized `pylock.toml` format does not depend on one tool. In the future, other tools could
install files that uv generates, and uv could install files from other tools.

The `pylock.toml` format cannot represent all uv features. uv therefore continues to use `uv.lock`
for its project interface.

uv supports `pylock.toml` exports and the format in the `uv pip` CLI:

- `uv export -o pylock.toml` exports a `uv.lock` file in `pylock.toml` format.
- `uv pip compile requirements.in -o pylock.toml` generates a `pylock.toml` file from requirements.
- `uv pip sync pylock.toml` and `uv pip install -r pylock.toml` install packages from a
  `pylock.toml` file.
