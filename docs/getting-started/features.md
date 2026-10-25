# Features

uv supports Python development tasks from installing Python and running scripts to managing large
projects. Those projects can support multiple Python versions and platforms.

The uv interface has several groups of commands. You can use these groups separately or together.

## Python versions

Install and manage Python.

- `uv python install`: Install Python versions.
- `uv python list`: View available Python versions.
- `uv python find`: Find an installed Python version.
- `uv python pin`: Pin the current project to use a specific Python version.
- `uv python uninstall`: Uninstall a Python version.

Read the [guide on installing Python](../guides/install-python.md) to get started.

## Scripts

Run standalone Python scripts, such as `example.py`.

- `uv run`: Run a script.
- `uv add --script`: Add a dependency to a script.
- `uv remove --script`: Remove a dependency from a script.

Read the [guide on running scripts](../guides/scripts.md) to get started.

## Projects

Create and manage Python projects that contain a `pyproject.toml` file.

- `uv init`: Create a new Python project.
- `uv add`: Add a dependency to the project.
- `uv remove`: Remove a dependency from the project.
- `uv sync`: Sync the project's dependencies with the environment.
- `uv lock`: Create a lockfile for the project's dependencies.
- `uv run`: Run a command in the project environment.
- `uv tree`: View the dependency tree for the project.
- `uv build`: Build the project into distribution archives.
- `uv publish`: Publish the project to a package index.

Read the [guide on projects](../guides/projects.md) to get started.

## Tools

Run and install tools from Python package indexes, such as `ruff` and `black`.

- `uvx` / `uv tool run`: Run a tool in a temporary environment.
- `uv tool install`: Install a tool user-wide.
- `uv tool uninstall`: Uninstall a tool.
- `uv tool list`: List installed tools.
- `uv tool update-shell`: Update the shell to include tool executables.

Read the [guide on tools](../guides/tools.md) to get started.

## The pip interface

Use the pip interface to manage environments and packages directly. It supports legacy workflows and
cases that need more control.

Create virtual environments with a replacement for `venv` and `virtualenv`:

- `uv venv`: Create a new virtual environment.

Read the documentation on [using environments](../pip/environments.md) for details.

Manage packages with replacements for [`pip`](https://github.com/pypa/pip) and
[`pipdeptree`](https://github.com/tox-dev/pipdeptree):

- `uv pip install`: Install packages into the current environment.
- `uv pip show`: Show details about an installed package.
- `uv pip freeze`: List installed packages and their versions.
- `uv pip check`: Check that the current environment has compatible packages.
- `uv pip list`: List installed packages.
- `uv pip uninstall`: Uninstall packages.
- `uv pip tree`: View the dependency tree for the environment.

Read the documentation on [managing packages](../pip/packages.md) for details.

Lock packages with a replacement for [`pip-tools`](https://github.com/jazzband/pip-tools):

- `uv pip compile`: Compile requirements into a lockfile.
- `uv pip sync`: Sync an environment with a lockfile.

Read the documentation on [locking environments](../pip/compile.md) for details.

!!! important

    These commands do not exactly match the interfaces or behavior of the tools they replace. Less
    common workflows are more likely to behave differently. Read the
    [pip-compatibility guide](../pip/compatibility.md) for details.

## Utility

Manage the uv cache and storage directories, inspect their locations, or update uv:

- `uv cache clean`: Remove cache entries.
- `uv cache prune`: Remove outdated cache entries and all centralized project environments.
- `uv cache dir`: Show the uv cache directory path.
- `uv tool dir`: Show the uv tool directory path.
- `uv python dir`: Show the uv installed Python versions path.
- `uv self update`: Update uv to the latest version.

## Next steps

Read the [guides](../guides/index.md) for an introduction to each feature. See the
[concept](../concepts/index.md) pages for detailed explanations. Learn how to [get help](./help.md)
if you have problems.
