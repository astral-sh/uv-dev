# Preview features

uv includes opt-in preview features. Community feedback helps confirm that a feature benefits users
before uv enables it for everyone.

## Enabling preview features

To enable all preview features, use the `--preview` flag:

```console
$ uv run --preview ...
```

Alternatively, set the `UV_PREVIEW` environment variable:

```console
$ UV_PREVIEW=1 uv run ...
```

To enable specific preview features, use the `--preview-features` flag:

```console
$ uv run --preview-features foo ...
```

To enable multiple features, repeat the `--preview-features` flag:

```console
$ uv run --preview-features foo --preview-features bar ...
```

Alternatively, specify the features in a comma-separated list:

```console
$ uv run --preview-features foo,bar ...
```

The `UV_PREVIEW_FEATURES` environment variable also accepts a comma-separated list:

```console
$ UV_PREVIEW_FEATURES=foo,bar uv run ...
```

To enable preview features in configuration, set them in `uv.toml` or under `[tool.uv]` in
`pyproject.toml` and PEP 723 metadata:

```toml
preview-features = ["foo", "bar"]
```

Set `preview-features = true` to enable all preview features.

Some preview features take effect before uv loads configuration files. Configuration files cannot
enable these features.

If a specified preview feature does not exist, uv warns but does not fail. This behavior preserves
backward compatibility for all configuration sources.

## Using preview features

Some preview features do not require a preview setting if a user action explicitly selects the
feature. For example, `uv pip install` accepts a `pylock.toml` file while `pylock.toml` support is
in preview. The specified file indicates that the user wants this feature. uv displays a warning
that the feature is in preview. Enable the preview feature to hide the warning.

## Available preview features

The following preview features are available:

--8<-- "docs/reference/.preview-features.md"

## Disabling preview features

The `--no-preview` option disables preview features.
