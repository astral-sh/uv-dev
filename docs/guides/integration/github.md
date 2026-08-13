---
title: Using uv in GitHub Actions
description:
  Install uv, set up Python, install dependencies, and publish packages in GitHub Actions.
---

# Using uv in GitHub Actions

## Installation

Use the official [`astral-sh/setup-uv`](https://github.com/astral-sh/setup-uv) action to install uv
in GitHub Actions. The action adds uv to `PATH` and can save the cache between runs. It supports all
platforms that uv supports.

To install the latest version of uv, use this configuration:

```yaml title="example.yml" hl_lines="11 12"
name: Example

jobs:
  uv-example:
    name: python
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

      - name: Install uv
        uses: astral-sh/setup-uv@c771a70e6277c0a99b617c7a806ffedaca235ff9 # v9.0.0
```

Pin uv to a specific version to make your workflow more predictable:

```yaml title="example.yml" hl_lines="14 15"
name: Example

jobs:
  uv-example:
    name: python
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

      - name: Install uv
        uses: astral-sh/setup-uv@c771a70e6277c0a99b617c7a806ffedaca235ff9 # v9.0.0
        with:
          # Install a specific version of uv.
          version: "0.12.10"
```

## Setting up Python

Run `uv python install` to install Python:

```yaml title="example.yml" hl_lines="14 15"
name: Example

jobs:
  uv-example:
    name: python
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

      - name: Install uv
        uses: astral-sh/setup-uv@c771a70e6277c0a99b617c7a806ffedaca235ff9 # v9.0.0

      - name: Set up Python
        run: uv python install
```

This command uses the Python version that the project pins.

You can also use the official GitHub `setup-python` action. This action can be faster because GitHub
caches Python versions with the runner.

Set the
[`python-version-file`](https://github.com/actions/setup-python/blob/main/docs/advanced-usage.md#using-the-python-version-file-input)
option to use the Python version that the project pins:

```yaml title="example.yml" hl_lines="14"
name: Example

jobs:
  uv-example:
    name: python
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

      - name: "Set up Python"
        uses: actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97 # v7.0.0
        with:
          python-version-file: ".python-version"

      - name: Install uv
        uses: astral-sh/setup-uv@c771a70e6277c0a99b617c7a806ffedaca235ff9 # v9.0.0
```

To ignore the pinned version, specify `pyproject.toml`. This installs the latest version that meets
the `requires-python` constraint of the project:

```yaml title="example.yml" hl_lines="14"
name: Example

jobs:
  uv-example:
    name: python
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

      - name: "Set up Python"
        uses: actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97 # v7.0.0
        with:
          python-version-file: "pyproject.toml"

      - name: Install uv
        uses: astral-sh/setup-uv@c771a70e6277c0a99b617c7a806ffedaca235ff9 # v9.0.0
```

## Multiple Python versions

When you use a matrix to test multiple Python versions, set the Python version with
`astral-sh/setup-uv`. This overrides the version in `pyproject.toml` or `.python-version`:

```yaml title="example.yml" hl_lines="17 18"
jobs:
  build:
    name: continuous-integration
    runs-on: ubuntu-latest
    strategy:
      matrix:
        python-version:
          - "3.10"
          - "3.11"
          - "3.12"

    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

      - name: Install uv and set the Python version
        uses: astral-sh/setup-uv@c771a70e6277c0a99b617c7a806ffedaca235ff9 # v9.0.0
        with:
          python-version: ${{ matrix.python-version }}
```

If you do not use the `setup-uv` action, set the `UV_PYTHON` environment variable:

```yaml title="example.yml" hl_lines="12"
jobs:
  build:
    name: continuous-integration
    runs-on: ubuntu-latest
    strategy:
      matrix:
        python-version:
          - "3.10"
          - "3.11"
          - "3.12"
    env:
      UV_PYTHON: ${{ matrix.python-version }}
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
```

## Syncing and running

After you install uv and Python, run `uv sync` to install the project. Use `uv run` to execute
commands in the project environment:

```yaml title="example.yml" hl_lines="15 17-22"
name: Example

jobs:
  uv-example:
    name: python
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

      - name: Install uv
        uses: astral-sh/setup-uv@c771a70e6277c0a99b617c7a806ffedaca235ff9 # v9.0.0

      - name: Install the project
        run: uv sync --locked --all-extras --dev

      - name: Run tests
        # Run the tests with pytest.
        run: uv run pytest tests
```

!!! tip

    Set
    [`UV_PROJECT_ENVIRONMENT`](../../concepts/projects/config.md#project-environment-path) to install
    packages in the system Python environment instead of creating a virtual environment.

## Caching

Save the uv cache between workflow runs to reduce CI time.

The [`astral-sh/setup-uv`](https://github.com/astral-sh/setup-uv) action can save the cache:

```yaml title="example.yml"
- name: Enable caching
  uses: astral-sh/setup-uv@c771a70e6277c0a99b617c7a806ffedaca235ff9 # v9.0.0
  with:
    enable-cache: true
```

To manage the cache yourself, use the `actions/cache` action:

```yaml title="example.yml"
jobs:
  install_job:
    env:
      # Set a fixed location for the uv cache.
      UV_CACHE_DIR: /tmp/.uv-cache

    steps:
      # ... set up Python and uv ...

      - name: Restore uv cache
        uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
        with:
          path: /tmp/.uv-cache
          key: uv-${{ runner.os }}-${{ hashFiles('uv.lock') }}
          restore-keys: |
            uv-${{ runner.os }}-${{ hashFiles('uv.lock') }}
            uv-${{ runner.os }}

      # ... install packages and run tests ...

      - name: Minimize uv cache
        run: uv cache prune --ci
```

Run `uv cache prune --ci` to reduce the cache size. This command is optimized for CI. Its effect on
performance depends on the packages that you install.

!!! tip

    If you use `uv pip`, use `requirements.txt` instead of `uv.lock` in the cache key.

!!! note

    [post-job-hook]: https://docs.github.com/en/actions/hosting-your-own-runners/managing-self-hosted-runners/running-scripts-before-or-after-a-job

    On persistent self-hosted runners, the default cache directory can grow without limit. To avoid
    sharing the cache between jobs, put it in the GitHub workspace. Use a
    [Post Job Hook][post-job-hook] to remove the cache after the job finishes.

    ```yaml
    install_job:
      env:
        # Set the uv cache location inside the GitHub workspace.
        UV_CACHE_DIR: ${{ github.workspace }}/.cache/uv
    ```

    To use a post-job hook, set `ACTIONS_RUNNER_HOOK_JOB_STARTED` on the self-hosted runner to the
    path of a cleanup script such as this script:

    ```sh title="clean-uv-cache.sh"
    #!/usr/bin/env sh
    uv cache clean
    ```

## Using `uv pip`

When you use the `uv pip` interface, uv requires a virtual environment by default. To install
packages in the system environment, add `--system` to each uv command or set `UV_SYSTEM_PYTHON`.

You can set `UV_SYSTEM_PYTHON` at different scopes.

To use the system environment for the entire workflow, set the variable at the top level:

```yaml title="example.yml"
env:
  UV_SYSTEM_PYTHON: 1

jobs: ...
```

To use the system environment for one job, set the variable for that job:

```yaml title="example.yml"
jobs:
  install_job:
    env:
      UV_SYSTEM_PYTHON: 1
    ...
```

To use the system environment for one step, set the variable for that step:

```yaml title="example.yml"
steps:
  - name: Install requirements
    run: uv pip install -r requirements.txt
    env:
      UV_SYSTEM_PYTHON: 1
```

Add `--no-system` to an individual uv command to disable this setting.

## Private repos

If your project [depends](../../concepts/projects/dependencies.md#git) on private GitHub
repositories, configure a [personal access token (PAT)][PAT]. This allows uv to download those
dependencies.

Create a PAT that has read access to the private repositories. Add the PAT as a [repository secret].

Use the [`gh`](https://cli.github.com/) CLI to configure a
[Git credential helper](../../concepts/authentication/git.md#git-credential-helpers). GitHub Actions
runners include `gh` by default. The helper uses the PAT for repositories that `github.com` hosts.

For example, if the repository secret is named `MY_PAT`, use this configuration:

```yaml title="example.yml"
steps:
  - name: Register the personal access token
    run: echo "${{ secrets.MY_PAT }}" | gh auth login --with-token
  - name: Configure the Git credential helper
    run: gh auth setup-git
```

[PAT]:
  https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens
[repository secret]:
  https://docs.github.com/en/actions/security-for-github-actions/security-guides/using-secrets-in-github-actions#creating-secrets-for-a-repository

## Publishing to PyPI

Use uv to build and publish a package to PyPI from GitHub Actions. For a complete example, see
[astral-sh/trusted-publishing-examples](https://github.com/astral-sh/trusted-publishing-examples).
The workflow uses [Trusted Publishing](https://docs.pypi.org/trusted-publishers/). You do not need
to configure long-lived credentials.

The example workflow uses a script to test the source distribution and wheel. The script checks that
both distributions work and contain the required files. This step is optional, but recommended.

!!! important

    This workflow uses separate `build` and `publish` jobs. Only the publishing job has
    `id-token: write`, which provides access to a publishing credential. The build job does not
    share this permission. This separation reduces the risk of supply chain attacks.

Add a release workflow to your project:

```yaml title=".github/workflows/release.yml"
name: "Publish release to PyPI"

on:
  push:
    tags:
      # Publish on version tags, such as v0.1.0.
      - "v[0-9]+.[0-9]+.[0-9]+"
      - "v[0-9]+.[0-9]+.[0-9]+rc[0-9]+"
      - "v[0-9]+.[0-9]+.[0-9]+[ab][0-9]+"

jobs:
  build:
    runs-on: ubuntu-latest
    permissions:
      contents: read
    steps:
      - name: Checkout
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false

      - name: Install uv
        uses: astral-sh/setup-uv@c771a70e6277c0a99b617c7a806ffedaca235ff9 # v9.0.0
        with:
          enable-cache: false

      - name: Build
        run: uv build

      # Optional, but recommended: run smoke tests on the distributions.
      - name: Smoke test (wheel)
        run: uv run --isolated --no-project --with dist/*.whl tests/smoke_test.py
      - name: Smoke test (source distribution)
        run: uv run --isolated --no-project --with dist/*.tar.gz tests/smoke_test.py

      - name: Upload distributions as artifacts
        uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1
        with:
          name: dist
          path: dist/

  publish:
    needs:
      - build
    runs-on: ubuntu-latest
    environment:
      name: pypi
    permissions:
      id-token: write
    steps:
      - name: Install uv
        uses: astral-sh/setup-uv@c771a70e6277c0a99b617c7a806ffedaca235ff9 # v9.0.0
        with:
          enable-cache: false

      - name: Download distributions artifact
        uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8.0.1
        with:
          name: dist
          path: dist/

      - name: Generate PEP 740 attestations
        uses: astral-sh/attest-action@f589a42a7efb6fe400b4f400de60b4bc90390027 # v0.0.6

      - name: Publish
        run: uv publish
```

In the GitHub repository, create the environment that the workflow defines. Open "Settings" ->
"Environments" to add the environment.

![GitHub settings dialog showing how to add the "pypi" environment under "Settings" -> "Environments"](../../assets/github-add-environment.png)

In the PyPI project settings, open "Publishing". Add a
[Trusted Publisher](https://docs.pypi.org/trusted-publishers/adding-a-publisher/) to the project.
Make sure that all fields match the GitHub configuration.

![PyPI project publishing settings dialog showing how to set all fields for a trusted publisher configuration](../../assets/pypi-add-trusted-publisher.png)

Save the configuration:

![PyPI project publishing settings dialog showing the configured trusted publishing settings](../../assets/pypi-with-trusted-publisher.png)

Tag a release and push the tag. The tag must start with `v` to match the workflow pattern.

```console
$ git tag -a v0.1.0 -m v0.1.0
$ git push --tags
```
