---
title: Using uv with mypy
description: A guide to running mypy with a project's dependencies and type stubs.
---

# Using uv with mypy

[mypy](https://mypy-lang.org/) is a static type checker for Python. To check a project's imports,
mypy needs access to its dependencies and their type information. Some libraries include type
information; others need a separate stub package, such as `types-requests` for `requests`.

## Using mypy in a project

In a [project](../../concepts/projects/index.md), add runtime dependencies normally, then add mypy
and any required stub packages as
[development dependencies](../../concepts/projects/dependencies.md#development-dependencies):

```console
$ uv add requests
$ uv add --dev mypy types-requests
```

This keeps the tools and dependencies in the same project environment. For example, the relevant
project configuration can look like this:

```toml title="pyproject.toml"
[project]
name = "mypy-example"
version = "0.1.0"
dependencies = ["requests"]

[dependency-groups]
dev = ["mypy", "types-requests"]
```

Create a file to check:

```python title="main.py"
import requests


def response_status(response: requests.Response) -> int:
    return response.status_code
```

Then run mypy in the project environment:

```console
$ uv run --group dev mypy main.py
```

The `dev` group is included by default, but specifying `--group dev` also works when the project's
[default groups](../../concepts/projects/dependencies.md#default-groups) have been customized.

## Using mypy as an isolated tool

[`uvx`](../../guides/tools.md) runs tools in isolated environments. Consequently, `uvx mypy main.py`
does not automatically make the project's dependencies or stub packages available to mypy. Prefer
`uv run` for checking a project. If you intentionally run mypy in a separate environment, consult
mypy's
[installed-package and PEP 561 guidance](https://mypy.readthedocs.io/en/stable/installed_packages.html#using-installed-packages-with-mypy-pep-561),
including its `--python-executable` option for selecting another Python environment.
