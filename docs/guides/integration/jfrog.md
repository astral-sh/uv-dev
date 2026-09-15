---
title: JFrog Artifactory
description: Install and publish Python packages with uv and JFrog Artifactory.
---

# JFrog Artifactory

Use a username and password or a JWT token to install packages from JFrog Artifactory.

Add the index to your project:

```toml title="pyproject.toml"
[[tool.uv.index]]
name = "private-registry"
url = "https://<organization>.jfrog.io/artifactory/api/pypi/<repository>/simple"
```

## Authenticate with username and password

```console
$ export UV_INDEX_PRIVATE_REGISTRY_USERNAME="<username>"
$ export UV_INDEX_PRIVATE_REGISTRY_PASSWORD="<password>"
```

## Authenticate with JWT token

```console
$ export UV_INDEX_PRIVATE_REGISTRY_USERNAME=""
$ export UV_INDEX_PRIVATE_REGISTRY_PASSWORD="$JFROG_JWT_TOKEN"
```

!!! note

    Replace `PRIVATE_REGISTRY` in the environment variable names with the index name in your
    `pyproject.toml`.

## Publishing packages

Add a `publish-url` to your index definition:

```toml title="pyproject.toml"
[[tool.uv.index]]
name = "private-registry"
url = "https://<organization>.jfrog.io/artifactory/api/pypi/<repository>/simple"
publish-url = "https://<organization>.jfrog.io/artifactory/api/pypi/<repository>"
```

!!! important

    The `--token "$JFROG_TOKEN"` option and `UV_PUBLISH_TOKEN` cause a 401 Unauthorized error with
    JFrog. JFrog requires an empty username, but uv uses `__token__` as the username with `--token`.

To authenticate, pass your token as the password and set the username to an empty string:

```console
$ uv publish --index <index_name> -u "" -p "$JFROG_TOKEN"
```

You can also set environment variables:

```console
$ export UV_PUBLISH_USERNAME=""
$ export UV_PUBLISH_PASSWORD="$JFROG_TOKEN"
$ uv publish --index private-registry
```

!!! note

    The publish environment variables, `UV_PUBLISH_USERNAME` and `UV_PUBLISH_PASSWORD`, do not include
    the index name.
