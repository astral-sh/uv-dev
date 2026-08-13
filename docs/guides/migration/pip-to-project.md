# Migrating from pip to a uv project

This guide explains how to move from a `pip` and `pip-tools` workflow based on `requirements` files
to a uv project workflow. The uv workflow uses `pyproject.toml` and `uv.lock` files.

!!! note

    Guides for moving from `pip` and `pip-tools` to the uv drop-in interface or migrating an existing
    `pyproject.toml` workflow are not available. Track progress in
    [#5200](https://github.com/astral-sh/uv/issues/5200).

This guide first explains `pip` workflows and then describes how to migrate to uv.

!!! tip

    If you understand the Python packaging ecosystem, go directly to the
    [requirements file import](#importing-requirements-files) instructions.

## Understanding pip workflows

### Project dependencies

Install a package before you use it in your project. Use `pip` to install a package directly:

```console
$ pip install fastapi
```

This command installs the package into the same environment as `pip`. That environment can be a
virtual environment or the global environment of your system Python installation.

Then run a Python script that imports the package:

```python title="example.py"
import fastapi
```

Create a separate virtual environment for each project to keep project packages separate. For
example:

```console
$ python -m venv
$ source .venv/bin/activate
$ pip ...
```

See the [project environments section](#project-environments) for more information.

### Requirements files

When you share a project, list its required packages in a file. `pip` can install those
requirements:

```requirements title="requirements.txt"
fastapi
```

```console
$ pip install -r requirements.txt
```

In this example, `fastapi` is not locked to a specific version. Different contributors can install
different versions. `pip-tools` helps keep versions consistent.

`pip-tools` uses separate requirements files to list project dependencies and lock their versions.
The file extensions identify each purpose. For example, list `fastapi` and `pydantic` in a
`requirements.in` file:

```requirements title="requirements.in"
fastapi
pydantic>2
```

`pydantic>2` allows only `pydantic` versions later than `2.0.0`. `fastapi` has no version
constraint, so any version is allowed.

Run the following command to compile these dependencies into `requirements.txt`:

```console
$ pip-compile requirements.in -o requirements.txt
```

```requirements title="requirements.txt"
annotated-types==0.7.0
    # via pydantic
anyio==4.8.0
    # via starlette
fastapi==0.115.11
    # via -r requirements.in
idna==3.10
    # via anyio
pydantic==2.10.6
    # via
    #   -r requirements.in
    #   fastapi
pydantic-core==2.27.2
    # via pydantic
sniffio==1.3.1
    # via anyio
starlette==0.46.1
    # via fastapi
typing-extensions==4.12.2
    # via
    #   fastapi
    #   pydantic
    #   pydantic-core
```

Each version constraint is _exact_, so each package has one permitted version. The example was
generated with `uv pip compile`. You can also generate it with `pip-compile` from `pip-tools`.

Alternatively, use `pip freeze` to generate `requirements.txt`. First install the input
dependencies, and then export the installed versions:

```console
$ pip install -r requirements.in
$ pip freeze > requirements.txt
```

```requirements title="requirements.txt"
annotated-types==0.7.0
anyio==4.8.0
fastapi==0.115.11
idna==3.10
pydantic==2.10.6
pydantic-core==2.27.2
sniffio==1.3.1
starlette==0.46.1
typing-extensions==4.12.2
```

After you lock the dependency versions, commit the requirements files to version control and
distribute them with the project.

Other users can then install the locked dependencies:

```console
$ pip install -r requirements.txt
```

<!--- TODO: Discuss equivalent commands for `uv pip compile` and `pip compile` -->

### Development dependencies

A requirements file can describe only one dependency group. Store additional _groups_, such as
development dependencies, in separate files. For example, create a `-dev` dependency file:

```requirements title="requirements-dev.in"
-r requirements.in
-c requirements.txt

pytest
```

`-r requirements.in` includes the base requirements, so the development environment considers _all_
dependencies together. `-c requirements.txt` _constrains_ package versions, so
`requirements-dev.txt` uses the same versions as `requirements.txt`.

!!! note

    You can use `-r requirements.txt` instead of both `-r requirements.in` and `-c requirements.txt`.
    Both approaches produce the same package versions. Using both files adds annotations that
    identify _direct_ dependencies with `-r requirements.in` and _indirect_ dependencies with
    `-c requirements.txt`.

The compiled development dependencies are:

```requirements title="requirements-dev.txt"
annotated-types==0.7.0
    # via
    #   -c requirements.txt
    #   pydantic
anyio==4.8.0
    # via
    #   -c requirements.txt
    #   starlette
fastapi==0.115.11
    # via
    #   -c requirements.txt
    #   -r requirements.in
idna==3.10
    # via
    #   -c requirements.txt
    #   anyio
iniconfig==2.0.0
    # via pytest
packaging==24.2
    # via pytest
pluggy==1.5.0
    # via pytest
pydantic==2.10.6
    # via
    #   -c requirements.txt
    #   -r requirements.in
    #   fastapi
pydantic-core==2.27.2
    # via
    #   -c requirements.txt
    #   pydantic
pytest==8.3.5
    # via -r requirements-dev.in
sniffio==1.3.1
    # via
    #   -c requirements.txt
    #   anyio
starlette==0.46.1
    # via
    #   -c requirements.txt
    #   fastapi
typing-extensions==4.12.2
    # via
    #   -c requirements.txt
    #   fastapi
    #   pydantic
    #   pydantic-core
```

Commit these files to version control and distribute them with the project. Contributors can install
the development requirements with:

```console
$ pip install -r requirements-dev.txt
```

### Platform-specific dependencies

`pip` and `pip-tools` compile dependencies for the current platform. The resulting file does not
necessarily work on other platforms, such as Windows or macOS.

For example, consider this dependency:

```requirements title="requirements.in"
tqdm
```

On Linux, this compiles to:

```requirements title="requirements-linux.txt"
tqdm==4.67.1
    # via -r requirements.in
```

On Windows, the same dependency compiles to:

```requirements title="requirements-win.txt"
colorama==0.4.6
    # via tqdm
tqdm==4.67.1
    # via -r requirements.in
```

`colorama` is a Windows-only dependency of `tqdm`.

If you use `pip` and `pip-tools`, create a locked requirements file for each supported platform.

!!! note

    The uv resolver can compile dependencies for multiple platforms at once. See
    ["universal resolution"](../../concepts/resolution.md#universal-resolution). This lets you use
    one `requirements.txt` file for every platform:

    ```console
    $ uv pip compile --universal requirements.in
    ```

    ```requirements title="requirements.txt"
    colorama==0.4.6 ; sys_platform == 'win32'
        # via tqdm
    tqdm==4.67.1
        # via -r requirements.in
    ```

    uv also uses universal resolution with `pyproject.toml` and `uv.lock`.

## Migrating to a uv project

### The `pyproject.toml`

`pyproject.toml` is the standard file for Python project metadata. It replaces `requirements.in`
files and supports multiple groups of project dependencies. It also stores project metadata, such as
the build system and tool settings.

<!-- TODO: Link to the official docs on this or write more -->

The `requirements.in` and `requirements-dev.in` examples correspond to this `pyproject.toml`:

```toml title="pyproject.toml"
[project]
name = "example"
version = "0.0.1"
dependencies = [
    "fastapi",
    "pydantic>2"
]

[dependency-groups]
dev = ["pytest"]
```

Later sections show how to import these files automatically.

### The uv lockfile

uv uses `uv.lock` to lock package versions. The file uses a uv-specific format that supports
advanced features and replaces `requirements.txt` files.

uv creates and updates the lockfile automatically when you add dependencies. To create it
explicitly, run `uv lock`.

`uv.lock` can contain multiple dependency groups. You do not need separate lockfiles for development
dependencies.

The uv lockfile is always [universal](../../concepts/resolution.md#universal-resolution). You do not
need separate files to [lock dependencies for each platform](#platform-specific-dependencies). This
keeps dependency versions consistent across supported platforms.

The uv lockfile also supports
[pinning packages to specific indexes](../../concepts/indexes.md#pinning-a-package-to-an-index),
which `requirements.txt` files cannot represent.

!!! tip

    To lock dependencies for only some platforms, use the
    [`tool.uv.environments`](../../concepts/resolution.md#limited-resolution-environments) setting
    to limit resolution and the lockfile.

See the [lockfile](../../concepts/projects/layout.md#the-lockfile) documentation for more
information.

### Importing requirements files

First, create a `pyproject.toml` if the project does not already have one:

```console
$ uv init
```

Then use `uv add` to import the requirements:

```console
$ uv add -r requirements.in
```

`requirements.in` does not pin exact package versions, so uv can select new versions. To keep the
versions already locked in `requirements.txt`, add that file as a _constraint_:

```console
$ uv add -r requirements.in -c requirements.txt
```

uv preserves the existing versions when it creates `uv.lock`.

#### Importing platform-specific constraints

You can migrate separate platform-specific dependency files to a universal lockfile. Do not pass the
existing files directly with `-c`. They do not contain environment markers, so their constraints can
conflict.

Use `uv pip compile` to add the required markers. For example, start with this file:

```requirements title="requirements-win.txt"
colorama==0.4.6
    # via tqdm
tqdm==4.67.1
    # via -r requirements.in
```

Add the markers with:

```console
$ uv pip compile requirements.in -o requirements-win.txt --python-platform windows --no-strip-markers
```

The updated file includes a Windows marker for `colorama`:

```requirements title="requirements-win.txt"
colorama==0.4.6 ; sys_platform == 'win32'
    # via tqdm
tqdm==4.67.1
    # via -r requirements.in
```

When you use `-o`, uv keeps the versions from the existing output file when possible.

To add markers for other platforms, change `--python-platform` and `-o` for each requirements file.
For example, use `linux` or `macos`.

After you add markers to each `requirements.txt` file, import the dependencies into `pyproject.toml`
and `uv.lock` with `uv add`:

```console
$ uv add -r requirements.in -c requirements-win.txt -c requirements-linux.txt
```

#### Importing development dependency files

The [development dependencies](#development-dependencies) section describes separate groups of
development dependencies.

Use `uv add --dev` to import development dependencies:

```console
$ uv add --dev -r requirements-dev.in -c requirements-dev.txt
```

If `requirements-dev.in` includes `requirements.in` through `-r`, remove that line first. Otherwise,
uv adds the base requirements to the `dev` dependency group. This example uses `sed` to remove lines
that start with `-r`, then pipes the result to `uv add`:

```console
$ sed '/^-r /d' requirements-dev.in | uv add --dev -r - -c requirements-dev.txt
```

uv also supports other dependency group names. For example, import documentation dependencies into a
`docs` group:

```console
$ uv add -r requirements-docs.in -c requirements-docs.txt --group docs
```

#### Importing dependency sources

A requirements file can include local paths and Git repositories:

```requirements title="requirements.in"
./path-dep
-e ./editable-path-dep
git-dep @ git+https://github.com/astral-sh/git-dep
```

uv maps these requirements to
[dependency sources](../../concepts/projects/dependencies.md#dependency-sources) in the
`[tool.uv.sources]` table in `pyproject.toml`:

```toml title="pyproject.toml"
[project]
dependencies = [
    "path-dep",
    "editable-path-dep",
    "git-dep",
]

[tool.uv.sources]
path-dep = { path = "./path-dep" }
editable-path-dep = { path = "./editable-path-dep", editable = true }
git-dep = { git = "https://github.com/astral-sh/git-dep" }
```

### Project environments

`pip` typically uses an active virtual environment. uv instead creates a dedicated virtual
environment in the `.venv` directory for each project. uv manages that environment automatically.
Commands such as `uv add` synchronize the environment with the project dependencies.

Use `uv run` to execute a command in the project environment:

```console
$ uv run pytest
```

Before each `uv run` command, uv verifies that the lockfile matches `pyproject.toml` and the
environment matches the lockfile. This keeps the project synchronized without manual steps. `uv run`
executes the command in a consistent, locked environment.

To create the project environment explicitly, run `uv sync`. This can help configure an editor.

!!! note

    By default, uv uses `.venv` in the project directory and ignores an active environment
    identified by `VIRTUAL_ENV`. To use the active environment, add the `--active` flag.

See the [project environment](../../concepts/projects/layout.md#the-project-environment)
documentation for more information.

## Next steps

After you migrate, see the [project concept](../../concepts/projects/index.md) page for more
information about uv projects.
