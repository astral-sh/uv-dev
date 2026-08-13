# Crates

## [uv-bench](./uv-bench)

Benchmarks uv.

## [uv-cache-key](./uv-cache-key)

Caches paths, URLs, and other resources across platforms.

## [uv-distribution-filename](./uv-distribution-filename)

Parses built distribution (wheel) and source distribution (sdist) filenames into structured
metadata.

## [uv-distribution-types](./uv-distribution-types)

Represents built distributions (wheels), source distributions (sdists), and their download sources.

## [uv-install-wheel-rs](./uv-install-wheel)

Installs built distributions (wheels) in a virtual environment.

## [uv-once-map](./uv-once-map)

Provides a [`waitmap`](https://github.com/withoutboats/waitmap)-like concurrent hash map that runs
each task exactly once.

## [uv-pep440-rs](./uv-pep440)

Handles Python version numbers and specifiers.

## [uv-pep508-rs](./uv-pep508)

Parses and evaluates
[dependency specifiers](https://packaging.python.org/en/latest/specifications/dependency-specifiers/),
previously known as [PEP 508](https://peps.python.org/pep-0508/).

## [uv-platform-tags](./uv-platform-tags)

Parses and infers Python platform tags defined by [PEP 425](https://peps.python.org/pep-0425/).

## [uv-cli](./uv-cli)

Implements the uv command-line interface.

## [uv-build-frontend](./uv-build-frontend)

Provides a [PEP 517](https://www.python.org/dev/peps/pep-0517/)-compatible build frontend for uv.

## [uv-cache](./uv-cache)

Caches Python packages and related metadata.

## [uv-client](./uv-client)

Connects to PyPI-compatible HTTP APIs.

## [uv-dev](./uv-dev)

Provides uv development tools.

## [uv-dispatch](./uv-dispatch)

Provides a central `struct` that resolves and builds source distributions in isolated environments.
Implements the traits defined in `uv-types`.

## [uv-distribution](./uv-distribution)

Works with built distributions (wheels) and source distributions (sdists). Fetches their metadata
and contents.

## [uv-extract](./uv-extract)

Extracts files from archives.

## [uv-fs](./uv-fs)

Provides filesystem utilities.

## [uv-git](./uv-git)

Works with Git repositories.

## [uv-installer](./uv-installer)

Installs Python packages in a virtual environment.

## [uv-python](./uv-python)

Detects and uses the current Python interpreter.

## [uv-netrc](./uv-netrc)

Provides a vendored netrc parser for uv.

## [uv-normalize](./uv-normalize)

Normalizes package and extra names according to Python specifications.

## [uv-requirements](./uv-requirements)

Reads package requirements from `pyproject.toml` and `requirements.txt` files.

## [uv-resolver](./uv-resolver)

Resolves Python packages and their dependencies.

## [uv-shell](./uv-shell)

Detects and modifies shell environments.

## [uv-types](./uv-types)

Defines shared traits to avoid circular dependencies.

## [uv-pypi-types](./uv-pypi-types)

Defines types for PyPI-compatible APIs.

## [uv-virtualenv](./uv-virtualenv)

Creates virtual environments in Rust as a replacement for `venv`.

## [uv-warnings](./uv-warnings)

Provides user-facing warnings for uv.

## [uv-workspace](./uv-workspace)

Defines uv workspace abstractions.

## [uv-requirements-txt](./uv-requirements-txt)

Parses `requirements.txt` files.
