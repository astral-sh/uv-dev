# Declaring dependencies

Declare dependencies in a static file instead of installing packages without a record. Then
[lock the dependencies](./compile.md) to create a consistent, reproducible environment.

## Using `pyproject.toml`

The `pyproject.toml` file is the Python standard for project configuration.

Define project dependencies in a `pyproject.toml` file:

```toml title="pyproject.toml"
[project]
dependencies = [
  "httpx",
  "ruff>=0.3.0"
]
```

Define optional dependencies in a `pyproject.toml` file:

```toml title="pyproject.toml"
[project.optional-dependencies]
cli = [
  "rich",
  "click",
]
```

Each key defines an "extra". Install extras with the `--extra` or `--all-extras` flag, or with
`package[<extra>]` syntax. See the documentation on
[installing packages](./packages.md#installing-packages-from-files) for details.

See the official
[`pyproject.toml` guide](https://packaging.python.org/en/latest/guides/writing-pyproject-toml/) for
more information about `pyproject.toml`.

## Using `requirements.in`

A [requirements file](https://pip.pypa.io/en/stable/reference/requirements-file-format/) can also
declare project dependencies. Put each requirement on a separate line. This file is usually named
`requirements.in` to distinguish it from `requirements.txt`, which contains locked dependencies.

Define dependencies in a `requirements.in` file:

```requirements title="requirements.in"
httpx
ruff>=0.3.0
```

This format does not support optional dependency groups.
