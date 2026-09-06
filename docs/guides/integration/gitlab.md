---
title: Using uv in GitLab CI/CD
description: Install uv, set up Python, and install dependencies in GitLab CI/CD.
---

# Using uv in GitLab CI/CD

## Using the uv image

Astral provides [Docker images](docker.md#available-images) with uv preinstalled. Select an image
that meets the requirements of your workflow.

```yaml title=".gitlab-ci.yml"
variables:
  UV_VERSION: "0.12.10"
  PYTHON_VERSION: "3.12"
  BASE_LAYER: trixie-slim
  # GitLab CI mounts the build directory separately. Copy files instead of using hard links.
  UV_LINK_MODE: copy

uv:
  image: ghcr.io/astral-sh/uv:$UV_VERSION-python$PYTHON_VERSION-$BASE_LAYER
  script:
    # Add your uv commands.
```

!!! note

    If you use a distroless image, specify the entrypoint:
    ```yaml
    uv:
      image:
        name: ghcr.io/astral-sh/uv:$UV_VERSION
        entrypoint: [""]
      # ...
    ```

## Caching

Save the uv cache between workflow runs to improve performance.

```yaml
uv-install:
  variables:
    UV_CACHE_DIR: .uv-cache
  cache:
    - key:
        files:
          - uv.lock
      paths:
        - $UV_CACHE_DIR
  script:
    # Add your uv commands.
  after_script:
    - uv cache prune --ci
```

For instructions on how to configure caching, see the
[GitLab caching documentation](https://docs.gitlab.com/ee/ci/caching/).

Run `uv cache prune --ci` at the end of the job to reduce the cache size. For more information, see
the [uv cache documentation](../../concepts/cache.md#caching-in-continuous-integration).

## Using `uv pip`

When you use the `uv pip` interface, uv requires a virtual environment by default. To install
packages in the system environment, add `--system` to each uv command or set the `UV_SYSTEM_PYTHON`
variable.

You can set `UV_SYSTEM_PYTHON` at different scopes. For information about variables and their
precedence, see the [GitLab CI/CD variables documentation](https://docs.gitlab.com/ee/ci/variables/).

To use the system environment for the entire workflow, set the variable at the top level:

```yaml title=".gitlab-ci.yml"
variables:
  UV_SYSTEM_PYTHON: 1

# [...]
```

Add `--no-system` to an individual uv command to disable this setting.

When you save the cache, use `requirements.txt` or `pyproject.toml` as the cache key files if you do
not use `uv.lock`.
