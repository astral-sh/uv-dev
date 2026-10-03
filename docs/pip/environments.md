# Using Python environments

Each Python installation has an environment that is active when Python runs. Install packages into
an environment to make their modules available to Python scripts. Avoid modifying a Python
installation's environment. This is especially important for operating-system Python installations,
which often manage their own packages. A virtual environment isolates packages from the Python
installation's environment. Unlike `pip`, uv requires a virtual environment by default.

## Creating a virtual environment

Create a virtual environment at `.venv`:

```console
$ uv venv
```

Specify a name or path to create a virtual environment at `my-name`:

```console
$ uv venv my-name
```

Request a Python version to create a virtual environment with Python 3.11:

```console
$ uv venv --python 3.11
```

uv downloads the requested Python version if it is not available on the system. See the
[Python version](../concepts/python-versions.md) documentation for details.

## Using a virtual environment

If the virtual environment uses its default name, uv automatically finds it for later commands.

```console
$ uv venv

$ # Install a package in the new virtual environment
$ uv pip install ruff
```

Activate the virtual environment to make its packages available:

=== "macOS and Linux"

    ```console
    $ source .venv/bin/activate
    ```

=== "Windows"

    ```pwsh-session
    PS> .venv\Scripts\activate
    ```

!!! note

    The default Unix activation script supports POSIX-compliant shells such as `sh`, `bash`, and
    `zsh`. Additional activation scripts support other common shells.

    === "fish"

        ```console
        $ source .venv/bin/activate.fish
        ```

    === "csh / tcsh"


        ```console
        $ source .venv/bin/activate.csh
        ```

    === "Nushell"

        ```console
        $ use .venv\Scripts\activate.nu
        ```

## Deactivating an environment

Use the `deactivate` command to exit a virtual environment:

```console
$ deactivate
```

## Using arbitrary Python environments

uv does not depend on Python, so it can install packages into other virtual environments. For
example, set `VIRTUAL_ENV=/path/to/venv` to install packages into `/path/to/venv`. The location of
the uv installation does not matter. uv ignores `VIRTUAL_ENV` if its directory is **not** a
[PEP 405-compliant](https://peps.python.org/pep-0405/#specification) virtual environment.

Use the `--python` option to install packages into any Python environment, including non-virtual
environments. For example, `uv pip install --python /path/to/python` installs packages into that
interpreter's environment. The `--python` option also accepts the root directory of a virtual
environment.

Use `uv pip install --system` to install packages into the system Python environment. This is
similar to `uv pip install --python $(which python)`, but uv skips executables from virtual
environments. Virtual environments are recommended for dependency management. The `--system` option
is appropriate in continuous integration and containerized environments.

The `--system` flag also allows uv to modify system environments. For example, use `--python 3.12`
to request a compatible Python interpreter. If uv finds a system interpreter such as
`/usr/lib/python3.12`, `--system` is required to modify its environment. Without `--system`, uv
ignores interpreters outside virtual environments. With `--system`, uv ignores interpreters inside
virtual environments.

System Python environments differ across platforms and distributions. uv supports common cases but
cannot install packages into every system environment. For example, uv does not support system
Python installations earlier than Python 3.10 on Debian. This is because Debian
[patches `distutils` but not `sysconfig`](https://ffy00.github.io/blog/02-python-debian-and-the-install-locations/).
Virtual environments are recommended in general and required for these non-standard environments.

If `pip` installs uv into a Python environment, uv can still modify other environments. However,
`python -m uv` uses the parent interpreter's environment by default. Running uv through Python adds
startup overhead and is not recommended for general use.

uv does not depend on Python. However, it needs a Python environment to install dependencies and
build source distributions.

## Discovery of Python environments

Commands that modify an environment, such as `uv pip sync` and `uv pip install`, search for a
virtual environment in this order:

- An activated virtual environment based on the `VIRTUAL_ENV` environment variable.
- An activated Conda environment based on the `CONDA_PREFIX` environment variable.
- A virtual environment at `.venv` in the current directory, or in the nearest parent directory.

If uv cannot find a virtual environment, it prompts you to create one with `uv venv`.

With `--system`, uv skips virtual environments and searches for an installed Python version.
Commands that do not modify an environment, such as `uv pip compile`, do not _require_ a virtual
environment. However, these commands still require a Python interpreter. See the documentation on
[Python discovery](../concepts/python-versions.md#discovery-of-python-versions) for details.
