# Building distributions

Publishing a project to an index such as PyPI requires a distribution.

Python projects usually provide source distributions (sdists) and binary distributions (wheels). An
sdist is usually a `.tar.gz` or `.zip` file. It contains the project's source code and additional
metadata. A wheel is a `.whl` file. It contains built artifacts that installers can use directly.

!!! important

    With `uv build`, uv acts as a [build frontend](https://peps.python.org/pep-0517/#terminology-and-goals).
    It selects the Python version and runs the build backend. The build backend defined in
    [`[build-system]`](./config.md#build-systems) controls the included files and distribution
    filenames. Each backend's documentation describes its build configuration.

## Using `uv build`

`uv build` creates source and binary distributions for a project. By default, it builds the project
in the current directory and writes distributions to `dist/`:

```console
$ uv build
$ ls dist/
example-0.1.0-py3-none-any.whl
example-0.1.0.tar.gz
```

A path argument selects a different project directory. For example, `uv build path/to/project`
builds the project at that path.

`uv build` first creates a source distribution. It then builds a binary distribution, or wheel, from
that source distribution.

`uv build --sdist` creates only a source distribution. `uv build --wheel` creates only a binary
distribution. `uv build --sdist --wheel` builds both distributions directly from source.

## Build constraints

`--build-constraint` limits the versions of build requirements. With `--require-hashes`, uv also
checks these requirements against specific known hashes. These checks help make builds reproducible.

For example, `constraints.txt` can define an exact build requirement and its hash:

```text
setuptools==68.2.2 --hash=sha256:b454a35605876da60632df1a60f736524eb73cc47bbc9f3f1ef1b644de74fd2a
```

The following command builds the project with the specified `setuptools` version. It also checks
that the downloaded `setuptools` distribution matches the specified hash:

```console
$ uv build --build-constraint constraints.txt --require-hashes
```

### Project build dependency hashes

Projects can also specify build constraints in
[`build-constraint-dependencies`](../../reference/settings.md#build-constraint-dependencies). To
verify a downloaded build dependency, provide its `requirement` and `hashes` in a table:

```toml
[tool.uv]
build-constraint-dependencies = [
    { requirement = "setuptools==68.2.2", hashes = ["sha256:b454a35605876da60632df1a60f736524eb73cc47bbc9f3f1ef1b644de74fd2a"] },
    "wheel<1",
]
```

An entry without hashes can be written as a string, as shown with `wheel<1` above. uv checks the
supplied hashes when it downloads pinned build dependencies during project resolution or
installation, including builds in `uv run --with` environments. These hashes apply to build
dependencies. Packages installed in the project environment are verified against their own lockfile
hashes.

## Preventing publish to PyPI

The `Private :: Do Not Upload` classifier marks a package as private:

```toml
[project]
classifiers = ["Private :: Do Not Upload"]
```

PyPI rejects uploads for packages with this classifier. The classifier does not change security or
privacy settings on other registries.

When only [per-project PyPI API tokens](https://pypi.org/help/#apitoken) are available, a project
cannot be published without its matching token.
