# Creating projects

`uv init` creates projects.

uv provides two project templates: [**applications**](#applications) and
[**libraries**](#libraries). By default, uv creates an application. `--lib` creates a library
instead.

For both templates, uv prefers to define a [build system](./config.md#build-systems). It places
source files in a dedicated `src/<project_name>/` directory. A build system supports Python
packaging features, such as command-line entry points. It also avoids common confusion with Python
imports. [`--no-package`](#creating-a-project-without-a-build-system) and
[`--bare`](#creating-a-minimal-project) can disable the build system.

!!! note

    Before v0.12, uv did not define a build system for applications by default.

## Target directory

By default, `uv init` creates a project in the working directory. A name selects a target directory,
as in `uv init foo`. `--directory` changes the working directory. uv then interprets the target path
relative to the selected working directory. If the target directory already contains a
`pyproject.toml` file, uv exits with an error.

## Applications

Application projects are suitable for web servers, scripts, and command-line interfaces.

`uv init` creates applications by default. `--app` also selects the application template:

```console
$ uv init example-app
```

uv places source code in a `src` directory with a module directory and an `__init__.py` file:

```console
$ tree example-app
example-app/
├── .python-version
├── README.md
├── pyproject.toml
└── src
    └── example_app
        └── __init__.py
```

The project defines a [build system](./config.md#build-systems). When the project environment is
synced, uv installs the project:

```toml title="pyproject.toml" hl_lines="12-14"
[project]
name = "example-app"
version = "0.1.0"
description = "Add your description here"
readme = "README.md"
requires-python = ">=3.11"
dependencies = []

[project.scripts]
example-app = "example_app:main"

[build-system]
requires = ["uv_build>=0.12.10,<0.13"]
build-backend = "uv_build"
```

!!! tip

    `--build-backend` selects an alternative build system.

`pyproject.toml` includes a [command](./config.md#entry-points) definition:

```toml title="pyproject.toml" hl_lines="9 10"
[project]
name = "example-app"
version = "0.1.0"
description = "Add your description here"
readme = "README.md"
requires-python = ">=3.11"
dependencies = []

[project.scripts]
example-app = "example_app:main"

[build-system]
requires = ["uv_build>=0.12.10,<0.13"]
build-backend = "uv_build"
```

`uv run` executes the command:

```console
$ cd example-app
$ uv run example-app
Hello from example-app!
```

## Libraries

A library provides functions and objects for other projects. Projects can distribute libraries
through services such as PyPI.

`--lib` creates a library:

```console
$ uv init --lib example-lib
```

!!! note

    Libraries always require a packaged project.

A `py.typed` marker indicates that other projects can read the library's types:

```console
$ tree example-lib
example-lib/
├── .python-version
├── README.md
├── pyproject.toml
└── src
    └── example_lib
        ├── py.typed
        └── __init__.py
```

!!! note

    A `src` layout isolates the library from `python` commands in the project root. It also
    separates distributed library code from other project source files.

The project defines a [build system](./config.md#build-systems). When the project environment is
synced, uv installs the project:

```toml title="pyproject.toml" hl_lines="12-14"
[project]
name = "example-lib"
version = "0.1.0"
description = "Add your description here"
readme = "README.md"
requires-python = ">=3.11"
dependencies = []

[build-system]
requires = ["uv_build>=0.12.10,<0.13"]
build-backend = "uv_build"
```

!!! tip

    `--build-backend` selects a different build backend template. Supported values include
    `hatchling`, `uv_build`, `flit-core`, `pdm-backend`, `setuptools`, `maturin`, and
    `scikit-build-core`. A [library with extension modules](#projects-with-extension-modules)
    requires an alternative backend.

The generated module defines an API function:

```python title="__init__.py"
def hello() -> str:
    return "Hello from example-lib!"
```

`uv run` can import the module and execute the function:

```console
$ cd example-lib
$ uv run python -c "import example_lib; print(example_lib.hello())"
Hello from example-lib!
```

## Projects with extension modules

Most Python projects are "pure Python". They do not define modules in languages such as C, C++,
FORTRAN, or Rust. Projects often use extension modules for performance-sensitive code.

An extension module requires an alternative build system. uv supports these build systems:

- [`maturin`](https://www.maturin.rs) for projects with Rust
- [`scikit-build-core`](https://github.com/scikit-build/scikit-build-core) for projects with C, C++,
  FORTRAN, Cython

`--build-backend` selects the build system:

```console
$ uv init --build-backend maturin example-ext
```

!!! note

    `--build-backend` implies `--package`.

With `maturin`, the project includes `Cargo.toml` and `lib.rs` in addition to standard Python
project files:

```console
$ tree example-ext
example-ext/
├── .python-version
├── Cargo.toml
├── README.md
├── pyproject.toml
└── src
    ├── lib.rs
    └── example_ext
        ├── __init__.py
        └── _core.pyi
```

!!! note

    With `scikit-build-core`, the project instead includes CMake configuration and a `main.cpp` file.

The Rust library defines a function:

```rust title="src/lib.rs"
use pyo3::prelude::*;

#[pymodule]
mod _core {
    use pyo3::prelude::*;

    #[pyfunction]
    fn hello_from_bin() -> String {
        "Hello from example-ext!".to_string()
    }
}
```

The Python module imports the function:

```python title="src/example_ext/__init__.py"
from example_ext._core import hello_from_bin


def main() -> None:
    print(hello_from_bin())
```

`uv run` executes the command:

```console
$ cd example-ext
$ uv run example-ext
Hello from example-ext!
```

!!! important

    When a project uses maturin or scikit-build-core, uv configures
    [`tool.uv.cache-keys`](../../reference/settings.md#cache-keys) to include common source file
    types. `--reinstall` forces a rebuild when changed files are outside `cache-keys` or no
    `cache-keys` are configured.

## Creating a project without a build system

A build system usually improves project development. Some projects instead define Python modules
directly in the top-level directory.

`--no-package` disables the build system:

```console
$ uv init --no-package example-app
```

The project includes `pyproject.toml`, a sample `main.py` file, a readme, and a `.python-version`
pin file.

```console
$ tree example-app
example-app/
├── .python-version
├── README.md
├── main.py
└── pyproject.toml
```

`pyproject.toml` contains basic metadata but does not define a build system. The project is not a
[package](./config.md#project-packaging), so uv does not install it into the environment:

```toml title="pyproject.toml"
[project]
name = "example-app"
version = "0.1.0"
description = "Add your description here"
readme = "README.md"
requires-python = ">=3.11"
dependencies = []
```

The sample file defines a `main` function and standard startup code:

```python title="main.py"
def main():
    print("Hello from example-app!")


if __name__ == "__main__":
    main()
```

`uv run` executes Python files:

```console
$ cd example-app
$ uv run main.py
Hello from example-app!
```

## Creating a minimal project

`--bare` creates a minimal project that contains only `pyproject.toml`:

```console
$ uv init example-bare --bare
```

uv does not create a Python version pin file, a README, or source directories and files. It also
does not initialize a version control system such as `git`.

```console
$ tree example-bare
example-bare
└── pyproject.toml
```

`pyproject.toml` also omits extra metadata such as `description` and `authors`.

```toml
[project]
name = "example-bare"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = []
```

`--bare` works with options such as `--lib` or `--build-backend`. In those cases, uv configures a
build system without creating the expected file structure.

Additional options can enable specific features with `--bare`:

```console
$ uv init example-bare --bare --description "Hello world" --author-from git --vcs git --pin-python
```
