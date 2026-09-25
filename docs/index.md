# uv

uv is an extremely fast Python package and project manager written in Rust.

<p align="center">
  <img alt="Shows a bar chart with benchmark results." src="https://github.com/astral-sh/uv/assets/1309177/629e59c0-9c6e-4013-9ad4-adb2bcf5080d#only-light">
</p>

<p align="center">
  <img alt="Shows a bar chart with benchmark results." src="https://github.com/astral-sh/uv/assets/1309177/03aa9163-1c79-4a87-a31d-7a9311ed9310#only-dark">
</p>

<p align="center">
  <i>Installing <a href="https://trio.readthedocs.io/">Trio</a>'s dependencies with a warm cache.</i>
</p>

## Highlights

- Replaces `pip`, `pip-tools`, `pipx`, `poetry`, `pyenv`, `twine`, `virtualenv`, and other tools.
- Runs [10-100x faster](https://github.com/astral-sh/uv/blob/main/BENCHMARKS.md) than `pip`.
- Manages [projects](#projects) with a
  [universal lockfile](./concepts/projects/layout.md#the-lockfile).
- [Runs scripts](#scripts) with
  [inline dependency metadata](./guides/scripts.md#declaring-script-dependencies).
- [Installs and manages](#python-versions) Python versions.
- [Runs and installs](#tools) command-line tools published as Python packages.
- Provides a faster [pip-compatible interface](#the-pip-interface) with familiar commands.
- Supports Cargo-style [workspaces](./concepts/projects/workspaces.md) for scalable projects.
- Saves disk space by reusing dependencies through a [global cache](./concepts/cache.md).
- Installs with `curl` or `pip`. The standalone installer does not require Rust or Python.
- Supports macOS, Linux, and Windows.

[Astral](https://astral.sh) supports uv and created [Ruff](https://github.com/astral-sh/ruff).

## Installation

Install uv with the official standalone installer:

=== "macOS and Linux"

    ```console
    $ curl -LsSf https://astral.sh/uv/install.sh | sh
    ```

=== "Windows"

    ```pwsh-session
    PS> powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"
    ```

Next, follow the [first steps](./getting-started/first-steps.md) or continue reading for an
overview.

!!! tip

    You can also install uv with pip, Homebrew, and other package managers. See the
    [installation page](./getting-started/installation.md) for all installation methods.

## Projects

uv manages project dependencies and environments. It supports lockfiles, workspaces, and other
features found in `rye` or `poetry`:

```console
$ uv init example
Initialized project `example` at `/home/user/example`

$ cd example

$ uv add ruff
Creating virtual environment at: .venv
Resolved 2 packages in 170ms
   Built example @ file:///home/user/example
Prepared 2 packages in 627ms
Installed 2 packages in 1ms
 + example==0.1.0 (from file:///home/user/example)
 + ruff==0.5.4

$ uv run ruff check
All checks passed!

$ uv lock
Resolved 2 packages in 0.33ms

$ uv sync
Resolved 2 packages in 0.70ms
Checked 1 package in 0.02ms
```

Read the [project guide](./guides/projects.md) to get started.

uv also builds and publishes projects that uv does not manage. Read the
[packaging guide](./guides/package.md) to learn more.

## Scripts

uv manages dependencies and environments for single-file scripts.

Create a script. Add inline metadata that declares its dependencies:

```console
$ echo 'import requests; print(requests.get("https://astral.sh"))' > example.py

$ uv add --script example.py requests
Updated `example.py`
```

Run the script in an isolated virtual environment:

```console
$ uv run example.py
Reading inline script metadata from: example.py
Installed 5 packages in 12ms
<Response [200]>
```

Read the [scripts guide](./guides/scripts.md) to get started.

## Tools

uv runs and installs command-line tools from Python packages, similar to `pipx`.

Run a tool in a temporary environment with `uvx`, an alias for `uv tool run`:

```console
$ uvx pycowsay 'hello world!'
Resolved 1 package in 167ms
Installed 1 package in 9ms
 + pycowsay==0.0.0.2
  """

  ------------
< hello world! >
  ------------
   \   ^__^
    \  (oo)\_______
       (__)\       )\/\
           ||----w |
           ||     ||
```

Install a tool with `uv tool install`:

```console
$ uv tool install ruff
Resolved 1 package in 6ms
Installed 1 package in 2ms
 + ruff==0.5.4
Installed 1 executable: ruff

$ ruff --version
ruff 0.5.4
```

Read the [tools guide](./guides/tools.md) to get started.

## Python versions

uv installs Python and lets you switch between versions.

Install several Python versions:

```console
$ uv python install 3.10 3.11 3.12
Searching for Python versions matching: Python 3.10
Searching for Python versions matching: Python 3.11
Searching for Python versions matching: Python 3.12
Installed 3 versions in 3.42s
 + cpython-3.10.14-macos-aarch64-none
 + cpython-3.11.9-macos-aarch64-none
 + cpython-3.12.4-macos-aarch64-none
```

Download Python versions when a command requires them:

```console
$ uv venv --python 3.12.0
Using CPython 3.12.0
Creating virtual environment at: .venv
Activate with: source .venv/bin/activate

$ uv run --python pypy@3.8 -- python
Python 3.8.16 (a9dbdca6fc3286b0addd2240f11d97d8e8de187a, Dec 29 2022, 11:45:30)
[PyPy 7.3.11 with GCC Apple LLVM 13.1.6 (clang-1316.0.21.2.5)] on darwin
Type "help", "copyright", "credits" or "license" for more information.
>>>>
```

Use a specific Python version in the current directory:

```console
$ uv python pin 3.11
Pinned `.python-version` to `3.11`
```

Read the [installing Python guide](./guides/install-python.md) to get started.

## The pip interface

uv provides replacements for common `pip`, `pip-tools`, and `virtualenv` commands.

uv adds features such as dependency version overrides and platform-independent resolution. It also
supports reproducible resolution and alternative resolution strategies.

Use `uv pip` to keep your existing workflows and run commands 10-100 times faster.

Compile requirements into a platform-independent requirements file:

```console
$ uv pip compile requirements.in \
   --universal \
   --output-file requirements.txt
Resolved 43 packages in 12ms
```

Create a virtual environment:

```console
$ uv venv
Using CPython 3.12.3
Creating virtual environment at: .venv
Activate with: source .venv/bin/activate
```

Install the locked requirements:

```console
$ uv pip sync requirements.txt
Resolved 43 packages in 11ms
Installed 43 packages in 208ms
 + babel==2.15.0
 + black==24.4.2
 + certifi==2024.7.4
 ...
```

Read the [pip interface documentation](./pip/index.md) to get started.

## Learn more

Follow the [first steps](./getting-started/first-steps.md) or read the [guides](./guides/index.md)
to start using uv.
