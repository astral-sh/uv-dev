# Running commands in projects

When working on a project, it is installed into the virtual environment at `.venv`. This environment
is isolated from the current shell by default, so invocations that require the project, e.g.,
`python -c "import example"`, will fail. Instead, use `uv run` to run commands in the project
environment:

```console
$ uv run python -c "import example"
```

When using `run`, uv will ensure that the project environment is up-to-date before running the given
command.

The given command can be provided by the project environment or exist outside of it, e.g.:

```console
$ # Presuming the project provides `example-cli`
$ uv run example-cli foo

$ # Running a `bash` script that requires the project to be available
$ uv run bash scripts/foo.sh
```

## Requesting additional dependencies

Additional dependencies or different versions of dependencies can be requested per invocation.

The `--with` option is used to include a dependency for the invocation, e.g., to request a different
version of `httpx`:

```console
$ uv run --with httpx==0.26.0 python -c "import httpx; print(httpx.__version__)"
0.26.0
$ uv run --with httpx==0.25.0 python -c "import httpx; print(httpx.__version__)"
0.25.0
```

The requested version will be respected regardless of the project's requirements. For example, even
if the project requires `httpx==0.24.0`, the output above would be the same.

### Importing wheels from the cache

To import an extracted wheel directly from uv's cache, pass its exact filename. The wheel must
already be cached from a configured index or `--find-links` location. uv checks its filename tags
and `Requires-Python` against the selected interpreter. A cache miss fails even when online.

```console
$ uv run --offline --with-cached-wheel idna-3.10-py3-none-any.whl python -Ic "import idna; print(idna.__file__)"
```

Repeat `--with-cached-wheel` for dependencies missing from the underlying environment. Imported
modules can point into the shared cache. Use this mode with code that treats its installed files as
read-only. Python suppresses automatic bytecode writes during imports after loading the wheel paths.
The option does not resolve wheel dependencies or generate console scripts. Wheels with a `.data`
directory require a regular installation. This mode uses Python site initialization, so `python -S`
skips the wheel paths.

## Running scripts

Scripts that declare inline metadata are automatically executed in environments isolated from the
project. See the [scripts guide](../../guides/scripts.md#declaring-script-dependencies) for more
details.

For example, given a script:

```python title="example.py"
# /// script
# dependencies = [
#   "httpx",
# ]
# ///

import httpx

resp = httpx.get("https://peps.python.org/api/peps.json")
data = resp.json()
print([(k, v["title"]) for k, v in data.items()][:10])
```

The invocation `uv run example.py` would run _isolated_ from the project with only the given
dependencies listed.

## Legacy scripts on Windows

Support is provided for
[legacy setuptools scripts](https://packaging.python.org/en/latest/guides/distributing-packages-using-setuptools/#scripts).
These types of scripts are additional files installed by setuptools in `.venv\Scripts`.

Currently only legacy scripts with the `.ps1`, `.cmd`, and `.bat` extensions are supported.

For example, to run a Command Prompt script:

```console
$ uv run --with nuitka==2.6.7 -- nuitka.cmd --version
```

In addition, you don't need to specify the extension. `uv` will automatically look for files ending
in `.ps1`, `.cmd`, and `.bat` in that order of execution on your behalf.

```console
$ uv run --with nuitka==2.6.7 -- nuitka --version
```

## Running from cached wheels

Enable the `run-cached-wheels` preview in a project to let `uv run` import dependencies directly
from uv's cache:

```toml title="pyproject.toml"
[tool.uv]
preview-features = ["run-cached-wheels"]
```

Run a command as usual, for example, `uv run pytest`. When the default project environment does not
exist and all selected packages have compatible, unpacked registry wheels in the cache, uv can
create a temporary environment with those wheels on Python's import path and installs their
command-line entry points. A cache miss, an editable project, or a wheel that needs file relocation
triggers a regular sync. uv uses an existing project environment normally. An explicitly isolated
run can also use cached wheels when the project has a `.venv`.

Lockfile updates, `--locked`, `--frozen`, extras, and dependency groups work as usual. Use a trusted
cache and keep it available for the command's lifetime. Python's `-S` option skips the wheel paths.
Package files are shared and should be treated as read-only by the command.

## Private project environments

Enable `run-private-environment` to run each project command in its own temporary virtual
environment:

```toml title="pyproject.toml"
[tool.uv]
preview-features = ["run-private-environment"]
```

For example, `uv run pytest` starts with its own temporary environment, even when the project
already has a `.venv`. Edits to installed wheel files are visible to other processes using that
environment and disappear when the run ends. When installing a private copy of the selected wheels,
uv requests filesystem clones and uses ordinary copies when cloning is unavailable or copying is
requested.

uv updates the lockfile as usual. Use `--locked` to require an up-to-date lockfile or `--frozen` to
run with the versions it already records. This preview takes precedence over `run-cached-wheels`. It
requires synchronization and currently rejects `--no-sync` and additional run requirements
(`--with`, `--with-editable`, `--with-requirements`, and `--with-cached-wheel`). Files in the
project working directory and editable source trees remain writable in their usual locations.

On Linux, uv can reuse a prepared environment for repeated private runs when it has effective
`CAP_SYS_ADMIN` and `CAP_SYS_CHROOT` and can create and join a private mount namespace with an
OverlayFS mount. Each run using a prepared environment gets its own temporary path and a writable
in-memory layer. On the first eligible run, uv prepares the selected wheels. Later eligible runs
with the same interpreter and wheel selection can reuse those files, including installed console
scripts.

This optimization requires a persistent cache. Every distribution selected for installation must be
a registry wheel with a recorded hash in the lockfile. Runs that need to build or refresh packages,
compile bytecode, or use `--link-mode copy` use the regular private installation. uv also uses that
installation when the runner cannot create the mounts.

## Signal handling

uv does not cede control of the process to the spawned command in order to provide better error
messages on failure. Consequently, uv is responsible for forwarding some signals to the child
process the requested command runs in.

On Unix systems, uv will forward most signals (with the exception of SIGKILL, SIGCHLD, SIGIO, and
SIGPOLL) to the child process. Since terminals send SIGINT to the foreground process group on
Ctrl-C, uv will only forward a SIGINT to the child process if it is sent more than once or the child
process group differs from uv's.

On Windows, these concepts do not apply and uv ignores Ctrl-C events, deferring handling to the
child process so it can exit cleanly.
