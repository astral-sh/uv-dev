# uv

<a href="https://pypi.python.org/pypi/uv"><img src="https://img.shields.io/pypi/v/uv.svg" alt="Latest PyPI version" /></a>
<a href="https://pypi.python.org/pypi/uv"><img src="https://img.shields.io/pypi/pyversions/uv.svg" alt="Supported Python versions" /></a>
<a href="https://discord.gg/astral-sh"><img src="https://img.shields.io/badge/Discord-%235865F2.svg?logo=discord&logoColor=white" alt="Discord" /></a>

An extremely fast Python package and project manager, written in Rust.

<p align="center">
  <picture align="center">
    <source media="(prefers-color-scheme: dark)" srcset="https://github.com/astral-sh/uv/assets/1309177/03aa9163-1c79-4a87-a31d-7a9311ed9310">
    <source media="(prefers-color-scheme: light)" srcset="https://github.com/astral-sh/uv/assets/1309177/629e59c0-9c6e-4013-9ad4-adb2bcf5080d">
    <img alt="Shows a bar chart with benchmark results." src="https://github.com/astral-sh/uv/assets/1309177/629e59c0-9c6e-4013-9ad4-adb2bcf5080d">
  </picture>
</p>

<p align="center">
  <i>Installing <a href="https://trio.readthedocs.io/">Trio</a>'s dependencies with a warm cache.</i>
</p>

## Highlights

- Replaces `pip`, `pip-tools`, `pipx`, `poetry`, `pyenv`, `twine`, `virtualenv`, and other tools.
- [10-100x faster](https://github.com/astral-sh/uv/blob/main/BENCHMARKS.md) than `pip`.
- [Manages projects](#projects) with a
  [universal lockfile](https://docs.astral.sh/uv/concepts/projects/layout#the-lockfile).
- [Runs scripts](#scripts) and supports
  [inline dependency metadata](https://docs.astral.sh/uv/guides/scripts#declaring-script-dependencies).
- [Installs and manages](#python-versions) Python versions.
- [Runs and installs](#tools) tools from Python packages.
- Includes a [pip-compatible interface](#the-pip-interface) to run familiar commands faster.
- Supports Cargo-style [workspaces](https://docs.astral.sh/uv/concepts/projects/workspaces) for
  scalable projects.
- Saves disk space with a [global cache](https://docs.astral.sh/uv/concepts/cache) that avoids
  duplicate dependency data.
- Installs with `curl` or `pip`. The standalone installer does not require Rust or Python.
- Supports macOS, Linux, and Windows.

uv is backed by [Astral](https://astral.sh), which created [Ruff](https://github.com/astral-sh/ruff)
and [ty](https://github.com/astral-sh/ty).

## Installation

Install uv with the standalone installer:

```bash
# On macOS and Linux.
curl -LsSf https://astral.sh/uv/install.sh | sh
```

```bash
# On Windows.
powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"
```

You can also install uv from [PyPI](https://pypi.org/project/uv/):

```bash
# With pip.
pip install uv
```

```bash
# Or pipx.
pipx install uv
```

If you used the standalone installer, update uv to the latest version with:

```bash
uv self update
```

See the [installation documentation](https://docs.astral.sh/uv/getting-started/installation/) for
more details and other installation methods.

## Documentation

Read the uv documentation at [docs.astral.sh/uv](https://docs.astral.sh/uv).

Run `uv help` to view the command-line reference documentation.

## Features

### Projects

Like `rye` and `poetry`, uv manages project dependencies and environments. It supports lockfiles,
workspaces, and other features:

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
 + ruff==0.5.0

$ uv run ruff check
All checks passed!

$ uv lock
Resolved 2 packages in 0.33ms

$ uv sync
Resolved 2 packages in 0.70ms
Checked 1 package in 0.02ms
```

See the [project documentation](https://docs.astral.sh/uv/guides/projects/) to get started.

uv also builds and publishes projects, including projects that it does not manage. See the
[publish guide](https://docs.astral.sh/uv/guides/publish/) for more information.

### Scripts

uv manages dependencies and environments for single-file scripts.

Create a script and declare its dependencies with inline metadata:

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

See the [scripts documentation](https://docs.astral.sh/uv/guides/scripts/) to get started.

### Tools

Like `pipx`, uv runs and installs command-line tools from Python packages.

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
 + ruff==0.5.0
Installed 1 executable: ruff

$ ruff --version
ruff 0.5.0
```

See the [tools documentation](https://docs.astral.sh/uv/guides/tools/) to get started.

### Python versions

uv installs Python versions and switches between them.

Install multiple Python versions:

```console
$ uv python install 3.12 3.13 3.14
Installed 3 versions in 972ms
 + cpython-3.12.12-macos-aarch64-none (python3.12)
 + cpython-3.13.9-macos-aarch64-none (python3.13)
 + cpython-3.14.0-macos-aarch64-none (python3.14)

```

Download Python versions as needed:

```console
$ uv venv --python 3.12.0
Using Python 3.12.0
Creating virtual environment at: .venv
Activate with: source .venv/bin/activate

$ uv run --python pypy@3.8 -- python --version
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

See the [Python installation documentation](https://docs.astral.sh/uv/guides/install-python/) to get
started.

### The pip interface

uv replaces common `pip`, `pip-tools`, and `virtualenv` commands.

It adds features such as dependency version overrides, platform-independent resolutions,
reproducible resolutions, and alternative resolution strategies.

Use the `uv pip` interface to keep your existing workflows and run them 10-100 times faster.

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
Using Python 3.12.3
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

See the [pip interface documentation](https://docs.astral.sh/uv/pip/index/) to get started.

## Contributing

Contributors of all experience levels can get involved. See the
[contributing guide](https://github.com/astral-sh/uv?tab=contributing-ov-file#contributing) to get
started.

## FAQ

#### How do you pronounce uv?

Pronounce uv as "you - vee" ([`/juː viː/`](https://en.wikipedia.org/wiki/Help:IPA/English#Key)).

#### How should I stylize uv?

Use "uv". See the [style guide](./STYLE.md#styling-uv) for details.

#### What platforms does uv support?

See uv's [platform support](https://docs.astral.sh/uv/reference/platforms/) document.

#### Is uv ready for production?

Yes. uv is stable and widely used in production. See its
[versioning policy](https://docs.astral.sh/uv/reference/versioning/) for details.

## Acknowledgements

The uv dependency resolver uses [PubGrub](https://github.com/pubgrub-rs/pubgrub). Thank you to the
PubGrub maintainers, especially [Jacob Finkelman](https://github.com/Eh2406), for their support.

uv bases its Git implementation on [Cargo](https://github.com/rust-lang/cargo).

Work in [pnpm](https://pnpm.io/), [Orogene](https://github.com/orogene/orogene), and
[Bun](https://github.com/oven-sh/bun) inspired some uv optimizations. uv also uses ideas from
Nathaniel J. Smith's [Posy](https://github.com/njsmith/posy) and adapts its
[trampoline](https://github.com/njsmith/posy/tree/main/src/trampolines/windows-trampolines/posy-trampoline)
for Windows support.

## License

uv is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in uv
by you, as defined in the Apache-2.0 license, shall be dually licensed as above, without any
additional terms or conditions.

<div align="center">
  <a target="_blank" href="https://astral.sh" style="background:none">
    <img src="https://raw.githubusercontent.com/astral-sh/uv/main/assets/svg/Astral.svg" alt="Made by Astral">
  </a>
</div>
