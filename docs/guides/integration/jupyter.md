---
title: Using uv with Jupyter
description:
  Use uv with Jupyter notebooks for interactive computing, data analysis, visualization, and
  environment management.
---

# Using uv with Jupyter

[Jupyter](https://jupyter.org/) notebooks support interactive computing, data analysis, and
visualization. Use Jupyter with uv to work in a project or run Jupyter as a standalone tool.

## Using Jupyter within a project

If you work in a [project](../../concepts/projects/index.md), start a Jupyter server with access to
the project environment:

```console
$ uv run --with jupyter jupyter lab
```

By default, `jupyter lab` starts the server at
[http://localhost:8888/lab](http://localhost:8888/lab).

In a notebook, import project modules as you would in other project files. For example, if the
project depends on `requests`, `import requests` imports the package from the project environment.

These steps are sufficient if you only need to read from the project environment. To install more
packages from the notebook, follow the guidance in the next sections.

### Creating a kernel

If you need to install packages from a notebook, create a dedicated kernel for your project. A
kernel lets the Jupyter server run in one environment while a notebook runs in another environment.

With uv, you can create a project kernel and install Jupyter in an isolated environment. For
example, use `uv run --with jupyter jupyter lab` to start Jupyter. The project kernel connects the
notebook to the correct environment. Packages that you install from the notebook go into the project
environment.

To create a kernel, add `ipykernel` as a development dependency:

```console
$ uv add --dev ipykernel
```

Create a kernel named `project`:

```console
$ uv run ipython kernel install --user --env VIRTUAL_ENV $(pwd)/.venv --name=project
```

Start the server:

```console
$ uv run --with jupyter jupyter lab
```

When you create a notebook, select the `project` kernel from the list. Use `!uv add pydantic` to add
`pydantic` to the project dependencies. Alternatively, use `!uv pip install pydantic` to install it
in the project environment without changing `pyproject.toml` or `uv.lock`. After either command, you
can run `import pydantic` in the notebook.

### Installing packages without a kernel

You can install packages from a notebook without a dedicated kernel. However, the installation
command determines which environment receives the package.

Although `uv run --with jupyter` runs in an isolated environment, `!uv add` modifies the _project_
environment. This behavior does not require a dedicated kernel.

For example, `!uv add pydantic` adds `pydantic` to the project dependencies and virtual environment.
You can then run `import pydantic` without more configuration or a server restart.

However, the Jupyter server provides the active environment for `!uv pip install`. This command
installs packages in _Jupyter's_ environment, not the project environment. These packages remain
available while the Jupyter server runs. They might not be available the next time you start it.

If a notebook requires pip, such as through the `%pip` magic, add pip to the project environment.
Run `uv venv --seed` before you start the Jupyter server:

```console
$ uv venv --seed
$ uv run --with jupyter jupyter lab
```

In the notebook, `%pip install` now installs packages in the project environment. However, these
changes do _not_ update `pyproject.toml` or `uv.lock`.

## Using Jupyter as a standalone tool

To use a notebook without a project, run `uv tool run jupyter lab`. This command starts a Jupyter
server in an isolated environment. You can use the notebook to run Python code interactively.

## Using Jupyter with a non-project environment

To run Jupyter in a virtual environment without a [project](../../concepts/projects/index.md),
install Jupyter directly in the environment. A non-project environment does not require
`pyproject.toml` or `uv.lock`:

=== "macOS and Linux"

    ```console
    $ uv venv --seed
    $ uv pip install pydantic
    $ uv pip install jupyterlab
    $ .venv/bin/jupyter lab
    ```

=== "Windows"

    ```pwsh-session
    PS> uv venv --seed
    PS> uv pip install pydantic
    PS> uv pip install jupyterlab
    PS> .venv\Scripts\jupyter lab
    ```

You can now run `import pydantic` in the notebook. Install more packages with `!uv pip install` or
`!pip install`.

## Using Jupyter from VS Code

You can use Jupyter notebooks in an editor such as VS Code. To connect a uv-managed project to a
Jupyter notebook in VS Code, create a project kernel:

```console
# Create a project.
$ uv init project

# Move into the project directory.
$ cd project

# Add ipykernel as a dev dependency.
$ uv add --dev ipykernel

# Open the project in VS Code.
$ code .
```

After you open the project in VS Code, select "Create: New Jupyter Notebook" from the command
palette. When VS Code prompts you to select a kernel, choose "Python Environments". Select the
project environment: `.venv/bin/python` on macOS and Linux, or `.venv\Scripts\python` on Windows.

!!! note

    VS Code requires `ipykernel` in the project environment. To install it without adding a
    development dependency, run `uv pip install ipykernel`.

To modify the project environment from the notebook, you might need to add `uv` as a development
dependency:

```console
$ uv add --dev uv
```

Use `!uv add pydantic` to add `pydantic` to the project dependencies. Alternatively, use
`!uv pip install pydantic` to install it in the project environment without changing
`pyproject.toml` or `uv.lock`.
