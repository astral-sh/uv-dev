---
title: Using uv in Docker
description:
  Use uv in Docker to manage Python dependencies, reduce build times, and control image size.
---

# Using uv in Docker

## Getting started

!!! tip

    For a complete example of how to build an application with uv and Docker, see the
    [`uv-docker-example`](https://github.com/astral-sh/uv-docker-example) project.

uv provides _distroless_ Docker images and images that use common base images. Distroless images
contain only the uv binaries. Use them to [copy uv binaries](#installing-uv) into your own images.
Derived images contain an operating system and a preinstalled copy of uv. Use them to run uv in a
container.

For example, run uv in a container that uses a Debian-based image:

```console
$ docker run --rm -it ghcr.io/astral-sh/uv:debian uv --help
```

### Available images

These distroless images are available:

- `ghcr.io/astral-sh/uv:latest`
- `ghcr.io/astral-sh/uv:{major}.{minor}.{patch}`, for example, `ghcr.io/astral-sh/uv:0.12.10`
- `ghcr.io/astral-sh/uv:{major}.{minor}`, for example, `ghcr.io/astral-sh/uv:0.12` (the latest patch
  version)

These derived images are available:

<!-- prettier-ignore-start -->

- Based on `alpine:3.23`:
    - `ghcr.io/astral-sh/uv:alpine`
    - `ghcr.io/astral-sh/uv:alpine3.23`
- Based on `alpine:3.22`:
    - `ghcr.io/astral-sh/uv:alpine3.22`
- Based on `debian:trixie-slim`:
    - `ghcr.io/astral-sh/uv:debian-slim`
    - `ghcr.io/astral-sh/uv:trixie-slim`
- Based on `buildpack-deps:trixie`:
    - `ghcr.io/astral-sh/uv:debian`
    - `ghcr.io/astral-sh/uv:trixie`
- Based on `dhi.io/alpine-base:3.23`:
    - `ghcr.io/astral-sh/uv:alpine-dhi`
    - `ghcr.io/astral-sh/uv:alpine3.23-dhi`
- Based on `dhi.io/debian-base:trixie-debian13`:
    - `ghcr.io/astral-sh/uv:debian-dhi`
    - `ghcr.io/astral-sh/uv:trixie-dhi`
- Based on `dhi/python:3.x`:
    - `ghcr.io/astral-sh/uv:python3.14-dhi`
    - `ghcr.io/astral-sh/uv:python3.13-dhi`
    - `ghcr.io/astral-sh/uv:python3.12-dhi`
    - `ghcr.io/astral-sh/uv:python3.11-dhi`
    - `ghcr.io/astral-sh/uv:python3.10-dhi`
- Based on `python3.x-alpine`:
    - `ghcr.io/astral-sh/uv:python3.15-rc-alpine`
    - `ghcr.io/astral-sh/uv:python3.15-rc-alpine3.23`
    - `ghcr.io/astral-sh/uv:python3.14-alpine`
    - `ghcr.io/astral-sh/uv:python3.14-alpine3.23`
    - `ghcr.io/astral-sh/uv:python3.13-alpine`
    - `ghcr.io/astral-sh/uv:python3.13-alpine3.23`
    - `ghcr.io/astral-sh/uv:python3.12-alpine`
    - `ghcr.io/astral-sh/uv:python3.12-alpine3.23`
    - `ghcr.io/astral-sh/uv:python3.11-alpine`
    - `ghcr.io/astral-sh/uv:python3.11-alpine3.23`
    - `ghcr.io/astral-sh/uv:python3.10-alpine`
    - `ghcr.io/astral-sh/uv:python3.10-alpine3.23`
    - `ghcr.io/astral-sh/uv:python3.9-alpine`
    - `ghcr.io/astral-sh/uv:python3.9-alpine3.22`
- Based on `python3.x-trixie`:
    - `ghcr.io/astral-sh/uv:python3.15-rc-trixie`
    - `ghcr.io/astral-sh/uv:python3.14-trixie`
    - `ghcr.io/astral-sh/uv:python3.13-trixie`
    - `ghcr.io/astral-sh/uv:python3.12-trixie`
    - `ghcr.io/astral-sh/uv:python3.11-trixie`
    - `ghcr.io/astral-sh/uv:python3.10-trixie`
    - `ghcr.io/astral-sh/uv:python3.9-trixie`
- Based on `python3.x-slim-trixie`:
    - `ghcr.io/astral-sh/uv:python3.15-rc-trixie-slim`
    - `ghcr.io/astral-sh/uv:python3.14-trixie-slim`
    - `ghcr.io/astral-sh/uv:python3.13-trixie-slim`
    - `ghcr.io/astral-sh/uv:python3.12-trixie-slim`
    - `ghcr.io/astral-sh/uv:python3.11-trixie-slim`
    - `ghcr.io/astral-sh/uv:python3.10-trixie-slim`
    - `ghcr.io/astral-sh/uv:python3.9-trixie-slim`

<!-- prettier-ignore-end -->

Each derived image also has uv version tags in the form
`ghcr.io/astral-sh/uv:{major}.{minor}.{patch}-{base}` and
`ghcr.io/astral-sh/uv:{major}.{minor}-{base}`. For example, use
`ghcr.io/astral-sh/uv:0.12.10-alpine`.

Starting with uv `0.8`, each derived image sets `UV_TOOL_BIN_DIR` to `/usr/local/bin`. This lets
`uv tool install` work with the default user.

For more information, see the
[GitHub Container Registry page](https://github.com/astral-sh/uv/pkgs/container/uv).

### Installing uv

Use an image that includes uv, or copy the binaries from the official distroless Docker image:

```dockerfile title="Dockerfile"
FROM python:3.12-slim-trixie
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/
```

You can also use the installer:

```dockerfile title="Dockerfile"
FROM python:3.12-slim-trixie

# Install curl and certificates so the installer can download the release archive.
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates

# Download the latest installer.
ADD https://astral.sh/uv/install.sh /uv-installer.sh

# Run the installer, then remove it.
RUN sh /uv-installer.sh && rm /uv-installer.sh

# Add the installed binary to PATH.
ENV PATH="/root/.local/bin/:$PATH"
```

The installer requires `curl`.

For either method, pin uv to a specific version:

```dockerfile
COPY --from=ghcr.io/astral-sh/uv:0.12.10 /uv /uvx /bin/
```

!!! tip

    The Dockerfile example pins an image tag. For reproducible builds, pin a specific SHA256 digest
    instead. A tag can point to a different image over time, but a digest identifies one image.

    ```Dockerfile
    # Use a digest from a previous release.
    COPY --from=ghcr.io/astral-sh/uv@sha256:2381d6aa60c326b71fd40023f921a0a3b8f91b14d5db6b90402e65a635053709 /uv /uvx /bin/
    ```

To pin the installer version, use this URL:

```dockerfile
ADD https://astral.sh/uv/0.12.10/install.sh /uv-installer.sh
```

### Installing a project

If uv manages your project, copy the project into the image and install it:

```dockerfile title="Dockerfile"
# Copy the project into the image.
COPY . /app

# Disable development dependencies.
ENV UV_NO_DEV=1

# Sync the project into a new environment and check that the lockfile is current.
WORKDIR /app
RUN uv sync --locked
```

!!! important

    Add `.venv` to the
    [`.dockerignore` file](https://docs.docker.com/build/concepts/context/#dockerignore-files) in
    your repository. This prevents Docker from including the local virtual environment in the
    image. The environment depends on the local platform, so create a new environment in the image.

To start your application by default, add this command:

```dockerfile title="Dockerfile"
# This example assumes that the project provides a my_app command.
CMD ["uv", "run", "my_app"]
```

!!! tip

    Use [intermediate layers](#intermediate-layers) to install dependencies separately from the
    project. This can reduce Docker image build times.

For a complete example, see the
[`uv-docker-example` project](https://github.com/astral-sh/uv-docker-example/blob/main/Dockerfile).

### Using the environment

After you install the project, _activate_ its virtual environment. Add the binary directory to the
start of `PATH`:

```dockerfile title="Dockerfile"
ENV PATH="/app/.venv/bin:$PATH"
```

You can also use `uv run` for commands that require the environment:

```dockerfile title="Dockerfile"
RUN uv run some_script.py
```

!!! tip

    To install packages in the system Python environment, set
    [`UV_PROJECT_ENVIRONMENT`](../../concepts/projects/config.md#project-environment-path) before you
    sync. You do not need to activate a virtual environment.

### Using installed tools

To use installed tools, add the [tool binary directory](../../concepts/tools.md#tool-executables) to
`PATH`:

```dockerfile title="Dockerfile"
ENV PATH=/root/.local/bin:$PATH
RUN uv tool install cowsay
```

```console
$ docker run -it $(docker build -q .) /bin/bash -c "cowsay -t hello"
  _____
| hello |
  =====
     \
      \
        ^__^
        (oo)\_______
        (__)\       )\/\
            ||----w |
            ||     ||
```

!!! note

    Run `uv tool dir --bin` in the container to find the tool binary directory.

    To set a fixed location, use this setting:

    ```dockerfile title="Dockerfile"
    ENV UV_TOOL_BIN_DIR=/opt/uv-bin/
    ```

## Developing in a container

For development, mount the project directory in a container. The containerized service then sees
project changes without an image rebuild. Do _not_ include the project virtual environment (`.venv`)
in the mount. Virtual environments depend on the platform, so the container must keep the
environment that the image provides.

### Mounting the project with `docker run`

Bind-mount the project directory to `/app`. Use an
[anonymous volume](https://docs.docker.com/engine/storage/#volumes) to keep the `.venv` directory in
the container:

```console
$ docker run --rm --volume .:/app --volume /app/.venv [...]
```

!!! tip

    The `--rm` flag removes the container and anonymous volume when the container exits.

For a complete example, see the
[`uv-docker-example` project](https://github.com/astral-sh/uv-docker-example/blob/main/run.sh).

### Configuring `watch` with `docker compose`

Docker Compose provides additional tools for container development. The
[`watch`](https://docs.docker.com/compose/file-watch/#compose-watch-versus-bind-mounts) option gives
you more control than a bind mount. It can update the containerized service when files change.

!!! note

    This feature requires Compose 2.22.0, which Docker Desktop 4.24 includes.

Configure `watch` in your
[Docker Compose file](https://docs.docker.com/compose/compose-application-model/#the-compose-file).
Sync the project directory without its virtual environment. Rebuild the image when the project
configuration changes:

```yaml title="compose.yaml"
services:
  example:
    build: .

    # ...

    develop:
      # Configure watch to update the application.
      #
      watch:
        # Sync the working directory with /app in the container.
        - action: sync
          path: .
          target: /app
          # Exclude the project virtual environment.
          ignore:
            - .venv/

        # Rebuild the image when pyproject.toml changes.
        - action: rebuild
          path: ./pyproject.toml
```

Run `docker compose watch` to start the container with this development configuration.

For a complete example, see the
[`uv-docker-example` project](https://github.com/astral-sh/uv-docker-example/blob/main/compose.yml).

## Optimizations

### Compiling bytecode

Compile Python source files to bytecode to reduce startup time in production images. This increases
installation time and image size.

To compile bytecode, add `--compile-bytecode`:

```dockerfile title="Dockerfile"
RUN uv python install --compile-bytecode
RUN uv sync --compile-bytecode
```

To compile bytecode for all commands in the Dockerfile, set `UV_COMPILE_BYTECODE`:

```dockerfile title="Dockerfile"
ENV UV_COMPILE_BYTECODE=1
```

!!! note

    During `uv python install`, uv compiles the standard library only for _managed_ Python versions.
    The distributor decides whether an unmanaged Python version includes a compiled standard
    library. For example, the official `python` image does not include a compiled standard library.

### Caching

Use a [cache mount](https://docs.docker.com/build/guide/mounts/#add-a-cache-mount) to improve
performance across builds:

```dockerfile title="Dockerfile"
ENV UV_LINK_MODE=copy

RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync
```

Set [`UV_LINK_MODE`](../../reference/settings.md#link-mode) to `copy` to prevent linking warnings.
The cache and sync target are on separate file systems, so uv cannot link files between them.

If you do not mount the cache, add `--no-cache` or set `UV_NO_CACHE` to reduce the image size.

By default, uv does not cache managed Python versions before it installs them. Set
`UV_PYTHON_CACHE_DIR` and use a cache mount to cache these versions:

```dockerfile title="Dockerfile"
ENV UV_PYTHON_CACHE_DIR=/root/.cache/uv/python

RUN --mount=type=cache,target=/root/.cache/uv \
    uv python install
```

!!! note

    Run `uv cache dir` in the container to find the cache directory.

    To set a fixed location for the cache, use this setting:

    ```dockerfile title="Dockerfile"
    ENV UV_CACHE_DIR=/opt/uv-cache/
    ```

### Intermediate layers

If uv manages your project, install its transitive dependencies in a separate layer. Use the
`--no-install` options to reduce build times.

The `uv sync --no-install-project` command installs the project dependencies, but not the project.
Projects usually change more often than their dependencies. A separate dependency layer lets Docker
reuse those dependencies between builds.

```dockerfile title="Dockerfile"
# Install uv.
FROM python:3.12-slim
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/

# Change the working directory to app.
WORKDIR /app

# Install the dependencies.
RUN --mount=type=cache,target=/root/.cache/uv \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    uv sync --locked --no-install-project

# Copy the project into the image.
COPY . /app

# Sync the project.
RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync --locked
```

The `pyproject.toml` file identifies the project root and name. Docker copies the project _contents_
into the image only before the final `uv sync` command.

!!! tip

    To exclude another package from the sync, use `--no-install-package <name>`.

#### Intermediate layers in workspaces

If you use a [workspace](../../concepts/projects/workspaces.md), make these changes:

- Use `--frozen` instead of `--locked` during the initial sync.
- Use `--no-install-workspace` to exclude the project _and_ all workspace members.

```dockerfile title="Dockerfile"
# Install uv.
FROM python:3.12-slim
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/

WORKDIR /app

RUN --mount=type=cache,target=/root/.cache/uv \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    uv sync --frozen --no-install-workspace

COPY . /app

RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync --locked
```

uv needs the `pyproject.toml` file for each workspace member to check whether `uv.lock` is current.
Use `--frozen` instead of `--locked` to skip this check during the first sync. After you copy all
workspace members into the image, use `--locked` for the next sync. This checks the lockfile against
all workspace members.

### Non-editable installs

By default, uv installs projects and workspace members in editable mode. Changes to the source code
are immediately available in the environment.

Both `uv sync` and `uv run` accept `--no-editable`. This option installs the project in non-editable
mode, so the installed project does not depend on its source directory.

For a multi-stage Docker image, use `--no-editable` to install the project in a virtual environment
in one stage. Copy only the virtual environment into the final image. You do not need to copy the
source code.

For example, use this Dockerfile:

```dockerfile title="Dockerfile"
# Install uv.
FROM python:3.12-slim AS builder
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/

# Use the system Python in both stages.
ENV UV_PYTHON_DOWNLOADS=0

# Change the working directory to app.
WORKDIR /app

# Install the dependencies.
RUN --mount=type=cache,target=/root/.cache/uv \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    uv sync --locked --no-install-project --no-editable

# Copy the project into the intermediate image.
COPY . /app

# Sync the project.
RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync --locked --no-editable

FROM python:3.12-slim

# Copy the environment without the source code.
COPY --from=builder /app/.venv /app/.venv

# Run the application.
CMD ["/app/.venv/bin/hello"]
```

### Using uv temporarily

If the final image does not require uv, mount the binary for each command:

```dockerfile title="Dockerfile"
RUN --mount=from=ghcr.io/astral-sh/uv,source=/uv,target=/bin/uv \
    uv sync
```

## Using the pip interface

### Installing a package

The system Python environment is safe to use because the container is already isolated. Add
`--system` to install packages in the system environment:

```dockerfile title="Dockerfile"
RUN uv pip install --system ruff
```

To use the system Python environment by default, set `UV_SYSTEM_PYTHON`:

```dockerfile title="Dockerfile"
ENV UV_SYSTEM_PYTHON=1
```

You can also create and activate a virtual environment:

```dockerfile title="Dockerfile"
RUN uv venv /opt/venv
# Use the virtual environment automatically.
ENV VIRTUAL_ENV=/opt/venv
# Add the environment entry points to the start of PATH.
ENV PATH="/opt/venv/bin:$PATH"
```

When you use a virtual environment, do not add `--system` to uv commands:

```dockerfile title="Dockerfile"
RUN uv pip install ruff
```

### Installing requirements

To install a requirements file, copy it into the container:

```dockerfile title="Dockerfile"
COPY requirements.txt .
RUN uv pip install -r requirements.txt
```

### Installing a project

When you install a project and its requirements, copy the requirements before the other source
files. This lets Docker cache the dependencies separately from the project. Dependencies usually
change less often than the project.

```dockerfile title="Dockerfile"
COPY pyproject.toml .
RUN uv pip install -r pyproject.toml
COPY . .
RUN uv pip install -e .
```

## Verifying image provenance

Astral signs its Docker images during the build. Use the image attestations to verify that an image
came from an official source.

For example, verify the attestations with the [GitHub CLI tool `gh`](https://cli.github.com/):

```console
$ gh attestation verify --owner astral-sh oci://ghcr.io/astral-sh/uv:latest
Loaded digest sha256:xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx for oci://ghcr.io/astral-sh/uv:latest
Loaded 1 attestation from GitHub API

The following policy criteria will be enforced:
- OIDC Issuer must match:................... https://token.actions.githubusercontent.com
- Source Repository Owner URI must match:... https://github.com/astral-sh
- Predicate type must match:................ https://slsa.dev/provenance/v1
- Subject Alternative Name must match regex: (?i)^https://github.com/astral-sh/

✓ Verification succeeded!

sha256:xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx was attested by:
REPO          PREDICATE_TYPE                  WORKFLOW
astral-sh/uv  https://slsa.dev/provenance/v1  .github/workflows/build-docker.yml@refs/heads/main
```

This verifies that the official uv GitHub release workflow built the Docker image. It also verifies
that the image has not changed since the build.

GitHub attestations use the [sigstore.dev infrastructure](https://www.sigstore.dev/). You can also
use [`cosign`](https://github.com/sigstore/cosign) to verify the attestation against the
multi-platform `uv` manifest:

```console
$ REPO=astral-sh/uv
$ gh attestation download --repo $REPO oci://ghcr.io/${REPO}:latest
Wrote attestations to file sha256:xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx.jsonl.
Any previous content has been overwritten

The trusted metadata is now available at sha256:xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx.jsonl
$ docker buildx imagetools inspect ghcr.io/${REPO}:latest --format "{{json .Manifest}}" > manifest.json
$ cosign verify-blob-attestation \
    --new-bundle-format \
    --bundle "$(jq -r .digest manifest.json).jsonl"  \
    --certificate-oidc-issuer="https://token.actions.githubusercontent.com" \
    --certificate-identity-regexp="^https://github\.com/${REPO}/.*" \
    <(jq -j '.|del(.digest,.size)' manifest.json)
Verified OK
```

!!! tip

    These examples use `latest`. For a stronger guarantee, verify a specific version tag, such as
    `ghcr.io/astral-sh/uv:0.12.10`. For the strongest guarantee, verify a specific image digest, such
    as `ghcr.io/astral-sh/uv:0.5.27@sha256:5adf09a5a526f380237408032a9308000d14d5947eafa687ad6c6a2476787b4f`.
