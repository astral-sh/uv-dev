# Troubleshooting build failures

uv builds a package when no compatible wheel, or pre-built package distribution, is available.
Package builds can fail for many reasons. Some failures do not relate to uv.

## Recognizing a build failure

The following example tries to install an old numpy version with a newer, unsupported Python
version:

```console
$ uv pip install -p 3.13 'numpy<1.20'
Resolved 1 package in 62ms
  × Failed to build `numpy==1.19.5`
  ├─▶ The build backend returned an error
  ╰─▶ Call to `setuptools.build_meta:__legacy__.build_wheel()` failed (exit status: 1)

      [stderr]
      Traceback (most recent call last):
        File "<string>", line 8, in <module>
          from setuptools.build_meta import __legacy__ as backend
        File "/home/konsti/.cache/uv/builds-v0/.tmpi4bgKb/lib/python3.13/site-packages/setuptools/__init__.py", line 9, in <module>
          import distutils.core
      ModuleNotFoundError: No module named 'distutils'

      hint: `distutils` was removed from the standard library in Python 3.12. Consider adding a constraint (like `numpy >1.19.5`) to avoid building a version of `numpy` that depends
      on `distutils`.
```

The error message includes "The build backend returned an error".

The failure includes `[stderr]` and, when present, `[stdout]` from the build backend. These error
logs come from the backend, not from uv.

The message after `╰─▶` describes the backend failure. uv may also provide a `hint:` to explain
common build failures, but not every failure includes a hint.

## Confirming that a build failure is specific to uv

Build failures usually relate to the system or build backend. Few are specific to uv. Running the
same installation with pip can show whether the failure also occurs with another installer:

```console
$ uv venv -p 3.13 --seed
$ source .venv/bin/activate
$ pip install --use-pep517 --no-cache --force-reinstall 'numpy==1.19.5'
Collecting numpy==1.19.5
  Using cached numpy-1.19.5.zip (7.3 MB)
  Installing build dependencies ... done
  Getting requirements to build wheel ... done
ERROR: Exception:
Traceback (most recent call last):
  ...
  File "/Users/example/.cache/uv/archive-v0/3783IbOdglemN3ieOULx2/lib/python3.13/site-packages/pip/_vendor/pyproject_hooks/_impl.py", line 321, in _call_hook
    raise BackendUnavailable(data.get('traceback', ''))
pip._vendor.pyproject_hooks._impl.BackendUnavailable: Traceback (most recent call last):
  File "/Users/example/.cache/uv/archive-v0/3783IbOdglemN3ieOULx2/lib/python3.13/site-packages/pip/_vendor/pyproject_hooks/_in_process/_in_process.py", line 77, in _build_backend
    obj = import_module(mod_path)
  File "/Users/example/.local/share/uv/python/cpython-3.13.0-macos-aarch64-none/lib/python3.13/importlib/__init__.py", line 88, in import_module
    return _bootstrap._gcd_import(name[level:], package, level)
           ~~~~~~~~~~~~~~~~~~~~~~^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
  File "<frozen importlib._bootstrap>", line 1387, in _gcd_import
  File "<frozen importlib._bootstrap>", line 1360, in _find_and_load
  File "<frozen importlib._bootstrap>", line 1310, in _find_and_load_unlocked
  File "<frozen importlib._bootstrap>", line 488, in _call_with_frames_removed
  File "<frozen importlib._bootstrap>", line 1387, in _gcd_import
  File "<frozen importlib._bootstrap>", line 1360, in _find_and_load
  File "<frozen importlib._bootstrap>", line 1331, in _find_and_load_unlocked
  File "<frozen importlib._bootstrap>", line 935, in _load_unlocked
  File "<frozen importlib._bootstrap_external>", line 1022, in exec_module
  File "<frozen importlib._bootstrap>", line 488, in _call_with_frames_removed
  File "/private/var/folders/6p/k5sd5z7j31b31pq4lhn0l8d80000gn/T/pip-build-env-vdpjme7d/overlay/lib/python3.13/site-packages/setuptools/__init__.py", line 9, in <module>
    import distutils.core
ModuleNotFoundError: No module named 'distutils'
```

!!! important

    The `--use-pep517` option gives `pip install` the same build isolation behavior as uv. uv uses
    [build isolation by default](../../pip/compatibility.md#pep-517-build-isolation).

    The `--force-reinstall` and `--no-cache` options help reproduce failures consistently.

Because this build also fails with pip, the issue is unlikely to be a uv bug.

When another installer reproduces the failure, the cause may be in an upstream project, such as
`numpy` or `setuptools`. Other solutions include avoiding the package build or installing its system
requirements.

## Why does uv build a package?

When creating a cross-platform lockfile, uv must determine the dependencies of every package. This
includes packages installed only on other platforms. During resolution, uv first checks for an
available wheel. If none exists, it looks for static metadata in the source distribution, such as
`pyproject.toml` fields or `METADATA` version 2.2 or later. It only builds the package when neither
source provides the required metadata.

During installation, uv needs a wheel for the current platform. If the index does not contain a
matching wheel, uv tries to build the source distribution.

The PyPI "Download Files" page lists the wheels for a project, for example,
https://pypi.org/project/numpy/2.1.1.md#files. A filename ending in `...-py3-none-any.whl` works
across platforms. Other wheel filenames include their supported operating system and platform. The
linked `numpy` example provides pre-built distributions for Python 3.10 to 3.13 on macOS, Linux, and
Windows.

## Common build failures

The following examples describe common build failures and their solutions.

### Command is not found

A build failure may report a missing command, such as `gcc`:

<!-- docker run --platform linux/x86_64 -it ghcr.io/astral-sh/uv:python3.10-trixie-slim /bin/bash -c "uv pip install --system pysha3==1.0.2" -->

```hl_lines="17"
× Failed to build `pysha3==1.0.2`
├─▶ The build backend returned an error
╰─▶ Call to `setuptools.build_meta:__legacy__.build_wheel` failed (exit status: 1)

    [stdout]
    running bdist_wheel
    running build
    running build_py
    creating build/lib.linux-x86_64-cpython-310
    copying sha3.py -> build/lib.linux-x86_64-cpython-310
    running build_ext
    building '_pysha3' extension
    creating build/temp.linux-x86_64-cpython-310/Modules/_sha3
    gcc -Wno-unused-result -Wsign-compare -DNDEBUG -g -fwrapv -O3 -Wall -fPIC -DPY_WITH_KECCAK=1 -I/root/.cache/uv/builds-v0/.tmp8V4iEk/include -I/usr/local/include/python3.10 -c
    Modules/_sha3/sha3module.c -o build/temp.linux-x86_64-cpython-310/Modules/_sha3/sha3module.o

    [stderr]
    error: command 'gcc' failed: No such file or directory
```

The system package manager can install the missing command. For the error above:

```console
$ apt install gcc
```

!!! tip

    uv-managed Python versions often require `clang` instead of `gcc`.

    Many Linux distributions provide a package with common build dependencies. On Debian or Ubuntu,
    `build-essential` provides most of these requirements:

    ```console
    $ apt install build-essential
    ```

### Header or library is missing

A build failure may report a missing header or library, such as a `.h` file. The system package
manager can install the missing dependency.

For example, `pygraphviz` requires Graphviz:

<!-- docker run --platform linux/x86_64 -it ghcr.io/astral-sh/uv:python3.12-trixie /bin/bash -c "uv pip install --system 'pygraphviz'" -->

```hl_lines="18-19"
× Failed to build `pygraphviz==1.14`
├─▶ The build backend returned an error
╰─▶ Call to `setuptools.build_meta.build_wheel` failed (exit status: 1)

  [stdout]
  running bdist_wheel
  running build
  running build_py
  ...
  gcc -fno-strict-overflow -Wsign-compare -DNDEBUG -g -O3 -Wall -fPIC -DSWIG_PYTHON_STRICT_BYTE_CHAR -I/root/.cache/uv/builds-v0/.tmpgLYPe0/include -I/usr/local/include/python3.12 -c pygraphviz/graphviz_wrap.c -o
  build/temp.linux-x86_64-cpython-312/pygraphviz/graphviz_wrap.o

  [stderr]
  ...
  pygraphviz/graphviz_wrap.c:9: warning: "SWIG_PYTHON_STRICT_BYTE_CHAR" redefined
      9 | #define SWIG_PYTHON_STRICT_BYTE_CHAR
        |
  <command-line>: note: this is the location of the previous definition
  pygraphviz/graphviz_wrap.c:3023:10: fatal error: graphviz/cgraph.h: No such file or directory
    3023 | #include "graphviz/cgraph.h"
        |          ^~~~~~~~~~~~~~~~~~~
  compilation terminated.
  error: command '/usr/bin/gcc' failed with exit code 1

  hint: This error likely indicates that you need to install a library that provides "graphviz/cgraph.h" for `pygraphviz@1.14`
```

On Debian, the `libgraphviz-dev` package resolves this error:

```console
$ apt install libgraphviz-dev
```

The `graphviz` package alone is not sufficient. The development headers are also required.

!!! tip

    The [`python3-dev` package](https://packages.debian.org/trixie/python3-dev) provides a missing
    `Python.h` header.

### Module is missing or cannot be imported

When a build fails because an import is missing,
[disabling build isolation](../../concepts/projects/config.md#build-isolation) may resolve the
issue.

For example, some packages assume that `pip` is available without declaring it as a build
dependency:

<!-- docker run --platform linux/x86_64 -it ghcr.io/astral-sh/uv:python3.12-trixie-slim /bin/bash -c "uv pip install --system chumpy" -->

```hl_lines="7"
  × Failed to build `chumpy==0.70`
  ├─▶ The build backend returned an error
  ╰─▶ Call to `setuptools.build_meta:__legacy__.build_wheel` failed (exit status: 1)

    [stderr]
    Traceback (most recent call last):
      File "<string>", line 9, in <module>
    ModuleNotFoundError: No module named 'pip'

    During handling of the above exception, another exception occurred:

    Traceback (most recent call last):
      File "<string>", line 14, in <module>
      File "/root/.cache/uv/builds-v0/.tmpvvHaxI/lib/python3.12/site-packages/setuptools/build_meta.py", line 334, in get_requires_for_build_wheel
        return self._get_build_requires(config_settings, requirements=[])
                ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
      File "/root/.cache/uv/builds-v0/.tmpvvHaxI/lib/python3.12/site-packages/setuptools/build_meta.py", line 304, in _get_build_requires
        self.run_setup()
      File "/root/.cache/uv/builds-v0/.tmpvvHaxI/lib/python3.12/site-packages/setuptools/build_meta.py", line 522, in run_setup
        super().run_setup(setup_script=setup_script)
      File "/root/.cache/uv/builds-v0/.tmpvvHaxI/lib/python3.12/site-packages/setuptools/build_meta.py", line 320, in run_setup
        exec(code, locals())
      File "<string>", line 11, in <module>
    ModuleNotFoundError: No module named 'pip'
```

Installing the build dependencies first and disabling build isolation for the package resolves this
error:

```console
$ uv pip install pip setuptools
$ uv pip install chumpy --no-build-isolation-package chumpy
```

The environment must contain the missing package, such as `pip`, _and_ all other build dependencies,
such as `setuptools`.

### Old version of the package is built

The resolver may try to build an old package version because of algorithmic limitations. A
[constraint](../settings.md#constraint-dependencies) with a lower bound, such as `numpy>=1.17`, can
prevent uv from selecting these versions.

For example, when resolving the following dependencies on Python 3.10, uv attempts to build an old
version of `apache-beam`.

```title="requirements.txt"
dill<0.3.9,>=0.2.2
apache-beam<=2.49.0
```

<!-- docker run --platform linux/x86_64 -it ghcr.io/astral-sh/uv:python3.10-trixie-slim /bin/bash -c "printf 'dill<0.3.9,>=0.2.2\napache-beam<=2.49.0' | uv pip compile -" -->

```hl_lines="1"
× Failed to build `apache-beam==2.0.0`
├─▶ The build backend returned an error
╰─▶ Call to `setuptools.build_meta:__legacy__.build_wheel` failed (exit status: 1)

    [stderr]
    ...
```

A lower-bound constraint, such as `apache-beam<=2.49.0,>2.30.0`, prevents uv from selecting an old
`apache-beam` version and resolves this build failure.

The `constraints.txt` file and [`constraint-dependencies`](../settings.md#constraint-dependencies)
setting also support constraints on indirect dependencies.

### Old Version of a build dependency is used

A build may fail when uv selects an incompatible or old build dependency. The
[`build-constraint-dependencies`](../settings.md#build-constraint-dependencies) setting or a
`build-constraints.txt` file limits the versions that uv can select for build dependencies.

For example, the issue in
[#5551](https://github.com/astral-sh/uv/issues/5551#issuecomment-2256055975) can be addressed by a
build constraint that excludes `setuptools` version `72.0.0`:

```toml title="pyproject.toml"
[tool.uv]
# Prevent setuptools version 72.0.0 from being used as a build dependency.
build-constraint-dependencies = ["setuptools!=72.0.0"]
```

This constraint prevents package builds from using the incompatible `setuptools` version.

### Package is only needed for an unused platform

Locking may fail while building a package for an unsupported platform.
[Limiting resolution](../../concepts/projects/config.md#limited-resolution-environments) to the
supported platforms can prevent this failure.

### Package does not support all Python versions

Environment markers can select different package versions for different Python versions. For
example, each `numpy` version supports only four Python minor versions. Supporting Python 3.8 to
3.13 requires separate `numpy` requirements:

```
numpy>=1.23; python_version >= "3.10"
numpy<1.23; python_version < "3.10"
```

### Package is only usable on a specific platform

Locking may fail when a package can only run on a different platform.
[Providing dependency metadata manually](../settings.md#dependency-metadata) avoids building the
package. uv cannot verify this metadata, so it must be correct.
