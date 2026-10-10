# Configuration files

uv supports persistent configuration files at the project and user levels.

uv searches the current directory and its parent directories for the nearest `pyproject.toml` or
`uv.toml` file.

!!! note

    The `tool` commands operate at the user level and ignore local configuration files. These
    commands read only user-level configuration, such as `~/.config/uv/uv.toml`, and system-level
    configuration, such as `/etc/uv/uv.toml`.

For a workspace, uv starts its search at the workspace root and ignores configuration in workspace
members. Because uv locks the workspace as one unit, all members share the same configuration.

If uv finds a `pyproject.toml` file, it reads configuration from the `[tool.uv]` table. To set a
persistent index URL, add the following to `pyproject.toml`:

```toml title="pyproject.toml"
[[tool.uv.index]]
url = "https://test.pypi.org/simple"
default = true
```

If the file does not contain this table, uv ignores it and continues to search parent directories.

uv also searches for `uv.toml` files. These files use the same structure without the `[tool.uv]`
prefix. For example:

```toml title="uv.toml"
[[index]]
url = "https://test.pypi.org/simple"
default = true
```

!!! note

    `uv.toml` takes precedence over `pyproject.toml`. If a directory contains both files, uv reads
    `uv.toml` and ignores the `[tool.uv]` section in `pyproject.toml`.

uv also finds `uv.toml` files in user-level and system-level
[configuration directories](../reference/storage.md#configuration-directories). On macOS and Linux,
the user-level path is `~/.config/uv/uv.toml`, and the system-level path is `/etc/uv/uv.toml`. On
Windows, these paths are `%APPDATA%\uv\uv.toml` and `%PROGRAMDATA%\uv\uv.toml`.

!!! important

    User- and system-level configuration files cannot use the `pyproject.toml` format.

If uv finds project-level, user-level, and system-level configuration files, it merges their
settings. Project-level configuration takes precedence over user-level configuration, which takes
precedence over system-level configuration. If uv finds multiple system-level files, it uses only
the first file. For example, `$XDG_CONFIG_DIRS/uv/uv.toml` takes precedence over `/etc/uv/uv.toml`.

If the project-level and user-level tables contain the same string, number, or boolean setting, uv
uses the project-level value. If both tables contain an array, uv joins the arrays and puts the
project-level settings first.

Environment variables take precedence over persistent configuration. Command-line settings take
precedence over both.

The `--no-config` argument disables discovery of all persistent configuration.

The `--config-file` argument accepts the path to a `uv.toml` file. If this argument is present, uv
uses the specified file instead of _any_ discovered configuration files, including user-level files.

## Settings

See the [settings reference](../reference/settings.md) for the available settings.

## Environment variable files

`uv run` can load environment variables from dotenv files, such as `.env`, `.env.local`, and
`.env.development`. It uses the [`dotenvy`](https://github.com/allan2/dotenvy) crate to load them.

To load a `.env` file from a specific path, set `UV_ENV_FILE` or pass `--env-file` to `uv run`.

For example, load environment variables from a `.env` file in the current directory:

```console
$ echo "MY_VAR='Hello, world!'" > .env
$ uv run --env-file .env -- python -c 'import os; print(os.getenv("MY_VAR"))'
Hello, world!
```

The `--env-file` flag accepts multiple files. Values in later files override values in earlier
files. To specify multiple files in `UV_ENV_FILE`, separate their paths with spaces. For example,
use `UV_ENV_FILE="/path/to/file1 /path/to/file2"`.

To disable dotenv files, set `UV_NO_ENV_FILE` to `1` or pass `--no-env-file` to `uv run`. These
options override `UV_ENV_FILE` and `--env-file`.

If the same variable exists in the environment and a `.env` file, the environment value takes
precedence.

## Configuring the pip interface

The [`[tool.uv.pip]`](../reference/settings.md#pip) section configures _only_ the `uv pip`
command-line interface. These settings do not apply to other `uv` commands. Many settings also have
top-level equivalents. These top-level settings _do_ apply to `uv pip` unless a `uv.pip` setting
overrides them.

The `uv.pip` settings match the pip interface. Separate settings preserve compatibility while global
settings can use different designs, such as `--no-build`.

For example, an `index-url` under `[tool.uv.pip]` affects only `uv pip` subcommands, such as
`uv pip install`. It does not affect `uv sync`, `uv lock`, or `uv run`:

```toml title="pyproject.toml"
[tool.uv.pip]
index-url = "https://test.pypi.org/simple"
```
