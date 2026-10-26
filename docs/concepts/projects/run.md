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

## Running a command in a disposable Linux VM

`uv vm run` boots a prepared x86-64 Linux image with QEMU. The command runs as root in `/app` with
`/app/.venv/bin` first on the guest's `PATH`. Writes to the root disk go to a per-invocation
overlay. The host receives the command's exit status through a separate block device. Standard
output and error share the guest's serial console.

```console
$ uv vm run --image project.ext4 --kernel vmlinuz -- python -c "import sys; print(sys.prefix)"
/app/.venv
```

Prepare a raw ext4 root containing the guest's Python installation and its shared libraries. Inside
that root, run `uv sync --locked --no-editable --link-mode copy` in `/app` to populate `/app/.venv`.
Include `/dev` as a mount point. Supply a kernel with built-in virtio PCI, virtio block, ext4,
devtmpfs automount, procfs, sysfs, and serial console support. The host needs `qemu-img` and
`qemu-system-x86_64` on `PATH`.

The default accelerator is TCG. Use `--accel kvm` on a compatible Linux host with access to
`/dev/kvm`. The guest runs with its own environment, without networking or host directory mounts,
and powers off when the command exits.
