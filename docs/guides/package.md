---
title: Building and publishing a package
description: Use uv to build Python packages and publish them to a package index, such as PyPI.
---

# Building and publishing a package

Use `uv build` to create source and binary distributions of Python packages. Use `uv publish` to
upload those distributions to a package registry.

## Preparing your project

Before you publish your project, make sure that you can package it for distribution.

If `pyproject.toml` does not include a `[build-system]` definition, `uv sync` does not build your
project. However, `uv build` uses the legacy setuptools build system instead.

!!! note

    Projects created with `uv init` include a `[build-system]` definition by default.

Configure a build system for your project. For details, see the
[project configuration](../concepts/projects/config.md#build-systems) documentation.

## Building your package

Build your package with `uv build`:

```console
$ uv build
```

By default, `uv build` builds the project in the current directory. It puts the resulting files in
the `dist/` subdirectory.

Use `uv build <SRC>` to build the package in a specific directory. Use
`uv build --package <PACKAGE>` to build a package in the current workspace.

!!! info

    By default, `uv build` uses `tool.uv.sources` to resolve dependencies from `build-system.requires`
    in `pyproject.toml`. Before you publish a package, run `uv build --no-sources`. This command
    checks that the package builds without `tool.uv.sources`. Other build tools, such as
    [`pypa/build`](https://github.com/pypa/build), do not use `tool.uv.sources`.

## Updating your version

Use `uv version` to update your package version before you publish it. To view the current version,
see the [project documentation](./projects.md#viewing-your-version).

To set an exact version, provide it as a positional argument:

```console
$ uv version 1.0.0
hello-world 0.7.0 => 1.0.0
```

To preview the change without updating the `pyproject.toml`, use the `--dry-run` flag:

```console
$ uv version 2.0.0 --dry-run
hello-world 1.0.0 => 2.0.0
$ uv version
hello-world 1.0.0
```

To increase a component of your package version, use the `--bump` option:

```console
$ uv version --bump minor
hello-world 1.2.3 => 1.3.0
```

The `--bump` option supports these version components: `major`, `minor`, `patch`, `stable`, `alpha`,
`beta`, `rc`, `post`, and `dev`. If you provide multiple components, uv applies them from largest
(`major`) to smallest (`dev`).

To set a component to a specific numeric value, use `--bump <component>=<value>`:

```console
$ uv version --bump patch --bump dev=66463664
hello-world 0.0.1 => 0.0.2.dev66463664
```

To change a stable version to a pre-release, increase a major, minor, or patch component. Also
increase the pre-release component:

```console
$ uv version --bump patch --bump beta
hello-world 1.3.0 => 1.3.1b1
$ uv version --bump major --bump alpha
hello-world 1.3.0 => 2.0.0a1
```

To update a pre-release version, increase the relevant pre-release component:

```console
$ uv version --bump beta
hello-world 1.3.0b1 => 1.3.0b2
```

To change a pre-release version to a stable version, use `stable` to remove the pre-release
component:

```console
$ uv version --bump stable
hello-world 1.3.1b2 => 1.3.1
```

!!! info

    By default, `uv version` locks and syncs the project after it changes the version. Use `--frozen`
    to prevent both actions. Use `--no-sync` to prevent only the sync.

## Publishing your package

!!! note

    To publish to PyPI from GitHub Actions, see the
    [GitHub Guide](integration/github.md#publishing-to-pypi).

Publish your package with `uv publish`:

```console
$ uv publish
```

Set a PyPI token with `--token` or `UV_PUBLISH_TOKEN`. Alternatively, set a username with
`--username` or `UV_PUBLISH_USERNAME`. Set the password with `--password` or `UV_PUBLISH_PASSWORD`.
A configured Trusted Publisher, such as GitHub Actions, does not require credentials to publish to
PyPI. Instead,
[add a trusted publisher to the PyPI project](https://docs.pypi.org/trusted-publishers/adding-a-publisher/).

When using trusted publishing, uv will attempt to invalidate the short-lived PyPI token after
publishing, even if publishing fails. This further reduces the exposure period for the short-lived
token, beyond its already short lifetime.

If invalidation fails, uv emits a warning without changing the publishing result. Tokens provided
explicitly with `--token` or `UV_PUBLISH_TOKEN` are not revoked.

!!! note

    PyPI does not accept account passwords. Generate a token instead. A token is equivalent to
    setting `--username __token__` and using the token as the password.

For a custom index in `[[tool.uv.index]]`, add `publish-url`. Then run `uv publish --index <name>`.
For example:

```toml
[[tool.uv.index]]
name = "testpypi"
url = "https://test.pypi.org/simple/"
publish-url = "https://test.pypi.org/legacy/"
explicit = true
```

!!! note

    The `uv publish --index <name>` command requires `pyproject.toml`. Add a checkout step to your
    publish CI job.

Although `uv publish` retries failed uploads, a failure can leave some files missing from the
registry. For PyPI, run the same command again. uv skips identical files that already exist.

For other registries, use `--check-url <index url>`. Provide the package index URL, not the
publishing URL. If you use `--index`, uv uses the index URL as the check URL. uv skips existing
identical files and handles simultaneous uploads. Existing files must exactly match the files that
uv uploads. This requirement prevents one package version from mixing artifacts built from different
project contents.

### Uploading attestations with your package

!!! note

    Some third-party package indexes do not support attestations and reject uploads that include
    them. If an upload fails, use `--no-attestations` or `UV_PUBLISH_NO_ATTESTATIONS` to disable
    attestations.

!!! tip

    The `uv publish` command does not generate attestations. Create them separately before you
    publish.

Use `uv publish` to upload [attestations](https://peps.python.org/pep-0740/) to registries that
support them, such as PyPI.

uv automatically finds attestations and matches them to their distributions. For example,
`uv publish` uploads these attestations with the matching distributions:

```console
$ ls dist/
hello_world-1.0.0-py3-none-any.whl
hello_world-1.0.0-py3-none-any.whl.publish.attestation
hello_world-1.0.0.tar.gz
hello_world-1.0.0.tar.gz.publish.attestation
```

## Installing your package

Use `uv run` to check that you can install and import the package:

```console
$ uv run --with <PACKAGE> --no-project -- python -c "import <PACKAGE>"
```

The `--no-project` flag prevents uv from installing the package from your local project directory.

!!! tip

    If you recently installed the package, use `--refresh-package <PACKAGE>` to avoid a cached
    version.

## Next steps

For details about building and publishing packages, see the
[PyPA guides](https://packaging.python.org/en/latest/guides/section-build-and-publish/).

To use uv with other software, see the [integration guides](./integration/index.md).
