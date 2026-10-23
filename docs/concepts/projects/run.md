# Running commands in projects

uv installs the project into the virtual environment at `.venv`. By default, the current shell
cannot access this isolated environment. Commands that require the project, such as
`python -c "import example"`, therefore fail outside it. `uv run` runs commands in the project
environment:

```console
$ uv run python -c "import example"
```

`uv run` updates the project environment when necessary before it runs the requested command.

The command can come from the project environment or from outside it:

```console
$ # Presuming the project provides `example-cli`
$ uv run example-cli foo

$ # Running a `bash` script that requires the project to be available
$ uv run bash scripts/foo.sh
```

## Requesting additional dependencies

Each invocation can request additional dependencies or different dependency versions.

`--with` adds a dependency to one invocation. For example, these commands request different versions
of `httpx`:

```console
$ uv run --with httpx==0.26.0 python -c "import httpx; print(httpx.__version__)"
0.26.0
$ uv run --with httpx==0.25.0 python -c "import httpx; print(httpx.__version__)"
0.25.0
```

uv uses the requested version even when the project requires a different one. For example, the
output remains the same if the project requires `httpx==0.24.0`.

## Running scripts

uv runs scripts with inline metadata in environments isolated from the project. The
[scripts guide](../../guides/scripts.md#declaring-script-dependencies) describes these environments.

For example, a script can declare these dependencies:

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

`uv run example.py` runs the script in an environment _isolated_ from the project. That environment
contains only the declared dependencies.

## Legacy scripts on Windows

uv supports
[legacy setuptools scripts](https://packaging.python.org/en/latest/guides/distributing-packages-using-setuptools/#scripts).
setuptools installs these scripts as additional files in `.venv\Scripts`.

uv currently supports legacy scripts with `.ps1`, `.cmd`, and `.bat` extensions.

The following command runs a Command Prompt script:

```console
$ uv run --with nuitka==2.6.7 -- nuitka.cmd --version
```

The extension is optional. uv searches for matching `.ps1`, `.cmd`, and `.bat` files in that order.

```console
$ uv run --with nuitka==2.6.7 -- nuitka --version
```

## Signal handling

uv retains control of the process so it can provide better error messages when a command fails. It
must therefore forward some signals to the child process that runs the command.

On Unix systems, uv forwards most signals to the child process. The exceptions are SIGKILL, SIGCHLD,
SIGIO, and SIGPOLL. Terminals send SIGINT to the foreground process group when Ctrl-C is pressed. uv
forwards SIGINT only if it receives that signal more than once or the child process group differs
from its own.

On Windows, uv ignores Ctrl-C events. The child process handles these events and can exit cleanly.
