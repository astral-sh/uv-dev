---
title: Using uv with FastAPI
description:
  Use uv with FastAPI to manage Python dependencies, run applications, and deploy with Docker.
---

# Using uv with FastAPI

[FastAPI](https://github.com/fastapi/fastapi) is a high-performance Python web framework. Use uv to
manage FastAPI projects, install dependencies, create environments, and run applications.

!!! note

    View the source code for this guide in the [uv-fastapi-example](https://github.com/astral-sh/uv-fastapi-example) repository.

## Migrating an existing FastAPI project

The sample application in the
[FastAPI documentation](https://fastapi.tiangolo.com/tutorial/bigger-applications/) has this
structure:

```plaintext
project
└── app
    ├── __init__.py
    ├── main.py
    ├── dependencies.py
    ├── routers
    │   ├── __init__.py
    │   ├── items.py
    │   └── users.py
    └── internal
        ├── __init__.py
        └── admin.py
```

To use uv with this application, run this command in the `project` directory:

```console
$ uv init --no-package
```

This command creates a `pyproject.toml` file for a
[flat, unpackaged project](../../concepts/projects/init.md#unpackaged-applications).

Add FastAPI as a dependency:

```console
$ uv add fastapi --extra standard
```

The project now has this structure:

```plaintext
project
├── pyproject.toml
└── app
    ├── __init__.py
    ├── main.py
    ├── dependencies.py
    ├── routers
    │   ├── __init__.py
    │   ├── items.py
    │   └── users.py
    └── internal
        ├── __init__.py
        └── admin.py
```

The `pyproject.toml` file contains settings like these:

```toml title="pyproject.toml"
[project]
name = "uv-fastapi-example"
version = "0.1.0"
description = "FastAPI project"
readme = "README.md"
requires-python = ">=3.12"
dependencies = [
    "fastapi[standard]",
]
```

Run the FastAPI application with:

```console
$ uv run fastapi dev
```

`uv run` resolves the project dependencies and creates `uv.lock` next to `pyproject.toml`. It then
creates a virtual environment and runs the command in that environment.

To test the application, open http://127.0.0.1:8000/?token=jessica in a web browser.

## Deployment

To deploy the FastAPI application with Docker, use this `Dockerfile`:

```dockerfile title="Dockerfile"
FROM python:3.12-slim

# Install uv.
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/

# Copy the application into the container.
COPY . /app

# Install the application dependencies.
WORKDIR /app
RUN uv sync --frozen --no-cache

# Run the application.
CMD ["/app/.venv/bin/fastapi", "run", "app/main.py", "--port", "80", "--host", "0.0.0.0"]
```

Build the Docker image with:

```console
$ docker build -t fastapi-app .
```

Run the Docker container on your computer with:

```console
$ docker run -p 8000:80 fastapi-app
```

Open http://127.0.0.1:8000/?token=jessica in your browser to confirm that the application runs
correctly.

!!! tip

    For more information about uv and Docker, see the [Docker guide](./docker.md).
