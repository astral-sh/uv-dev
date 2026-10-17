# Managing packages

## Installing a package

Install a package, such as Flask, into the virtual environment:

```console
$ uv pip install flask
```

Install a package with optional dependencies, such as Flask with the "dotenv" extra:

```console
$ uv pip install "flask[dotenv]"
```

Install multiple packages, such as Flask and Ruff:

```console
$ uv pip install flask ruff
```

Install a package with a version constraint, such as Ruff v0.2.0 or newer:

```console
$ uv pip install 'ruff>=0.2.0'
```

Install a specific package version, such as Ruff v0.3.0:

```console
$ uv pip install 'ruff==0.3.0'
```

Install a package from a local directory:

```console
$ uv pip install "ruff @ ./projects/ruff"
```

Install a package from GitHub:

```console
$ uv pip install "git+https://github.com/astral-sh/ruff"
```

Install a package from a specific GitHub reference:

```console
$ # Install a tag
$ uv pip install "git+https://github.com/astral-sh/ruff@v0.2.0"

$ # Install a commit
$ uv pip install "git+https://github.com/astral-sh/ruff@1fadefa67b26508cc59cf38e6130bde2243c929d"

$ # Install a branch
$ uv pip install "git+https://github.com/astral-sh/ruff@main"
```

See the [Git authentication](../concepts/authentication/git.md) documentation to install from a
private repository.

## Editable packages

Changes to an editable package's source code take effect without reinstalling the package.

Install the current project as an editable package:

```console
$ uv pip install -e .
```

Install a project in another directory as an editable package:

```console
$ uv pip install -e "ruff @ ./project/ruff"
```

## Installing packages from files

Install multiple packages from files in standard formats.

Install from a `requirements.txt` file:

```console
$ uv pip install -r requirements.txt
```

See the [`uv pip compile`](./compile.md) documentation for more information on `requirements.txt`
files.

Install from a `pyproject.toml` file:

```console
$ uv pip install -r pyproject.toml
```

Install from a `pyproject.toml` file with optional dependencies from the "foo" extra:

```console
$ uv pip install -r pyproject.toml --extra foo
```

Install from a `pyproject.toml` file with all optional dependencies enabled:

```console
$ uv pip install -r pyproject.toml --all-extras
```

Install a dependency group, such as `foo`, from the current project's `pyproject.toml`:

```console
$ uv pip install --group foo
```

Specify the project directory that contains the dependency groups:

```console
$ uv pip install --project some/path/ --group foo --group bar
```

Alternatively, specify a `pyproject.toml` path for each group:

```console
$ uv pip install --group some/path/pyproject.toml:foo --group other/pyproject.toml:bar
```

!!! note

    As in pip, `--group` flags do not apply to sources specified with flags such as `-r` or `-e`.
    For example, `uv pip install -r some/path/pyproject.toml --group foo` reads `foo` from
    `./pyproject.toml`, **not** `some/path/pyproject.toml`.

## Uninstalling a package

Uninstall a package, such as Flask:

```console
$ uv pip uninstall flask
```

Uninstall multiple packages, such as Flask and Ruff:

```console
$ uv pip uninstall flask ruff
```
