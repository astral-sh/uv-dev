# Getting help

## Help menus

View the help menu for `uv` with the `--help` flag:

```console
$ uv --help
```

View help for a specific command, such as `uv init`:

```console
$ uv init --help
```

The `--help` flag displays a short help menu. Use `uv help` to view the longer help menu:

```console
$ uv help
```

View the longer help menu for a specific command, such as `uv init`:

```console
$ uv help init
```

uv uses `less` or `more`, when available, to display long help output one screen at a time. Press
`q` to close the pager.

## Displaying verbose output

Use the `-v` flag to display verbose output for a command, such as `uv sync`:

```console
$ uv sync -v
```

Repeat the `-v` flag to show more detailed output:

```console
$ uv sync -vv
```

Verbose output often explains why uv behaves a particular way.

## Viewing the version

Check your uv version before you ask for help. A newer version might already fix the problem.

Check the installed version:

```console
$ uv self version
```

You can also use these commands:

```console
$ uv --version      # Same output as `uv self version`
$ uv -V             # Will not include the build commit and date
```

!!! note

    uv versions earlier than 0.7.0 use `uv version` instead of `uv self version`.

## Troubleshooting issues

Read the [troubleshooting guide](../reference/troubleshooting/index.md) for common issues.

## Open an issue on GitHub

Use the GitHub [issue tracker](https://github.com/astral-sh/uv/issues) to report bugs and request
features. Search for similar issues before you open a new issue.

## Chat on Discord

Ask questions, learn about uv, and meet other community members on the
[Astral Discord server](https://discord.com/invite/astral-sh).
