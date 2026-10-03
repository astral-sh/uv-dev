---
title: Google Artifact Registry
description: Install and publish Python packages with uv and Google Artifact Registry.
---

# Google Artifact Registry

Use an access token or the [`keyring`](https://github.com/jaraco/keyring) package to install
packages from [Google Artifact Registry](https://cloud.google.com/artifact-registry/docs).

!!! note

    This guide requires an installed and authenticated
    [`gcloud` CLI](https://cloud.google.com/sdk/gcloud).

To use Google Artifact Registry, add the index to your project:

```toml title="pyproject.toml"
[[tool.uv.index]]
name = "private-registry"
url = "https://<REGION>-python.pkg.dev/<PROJECT>/<REPOSITORY>/simple/"
```

## Authenticate with a Google access token

Use the "Basic" HTTP authentication scheme. Put the access token in the password field of the URL.
Set the username to `oauth2accesstoken`, or authentication fails.

Generate a token with `gcloud`:

```bash
export ARTIFACT_REGISTRY_TOKEN=$(
    gcloud auth application-default print-access-token
)
```

!!! note

    You might need to pass extra parameters, such as `--project`, to generate the token. This command
    is a basic example.

Set the index credentials:

```bash
export UV_INDEX_PRIVATE_REGISTRY_USERNAME=oauth2accesstoken
export UV_INDEX_PRIVATE_REGISTRY_PASSWORD="$ARTIFACT_REGISTRY_TOKEN"
```

!!! note

    Make sure `PRIVATE_REGISTRY` matches the index name in your `pyproject.toml`.

## Authenticate with `keyring` and `keyrings.google-artifactregistry-auth`

To authenticate to Google Artifact Registry, use the [`keyring`](https://github.com/jaraco/keyring)
package with the
[`keyrings.google-artifactregistry-auth` plugin](https://github.com/GoogleCloudPlatform/artifact-registry-python-tools).
Install both packages from a source other than Google Artifact Registry because authentication
requires them.

The `keyrings.google-artifactregistry-auth` plugin wraps
[gcloud CLI](https://cloud.google.com/sdk/gcloud) to generate short-lived access tokens. It securely
stores the tokens in the system keyring and refreshes expired tokens.

uv only supports the `keyring` package in
[subprocess mode](../../reference/settings.md#keyring-provider). The `keyring` executable must be in
the `PATH`. Install it globally or in the active environment. The `keyring` CLI requires the
username `oauth2accesstoken` in the URL.

```bash
# Pre-install keyring and Artifact Registry plugin from the public PyPI
uv tool install keyring --with keyrings.google-artifactregistry-auth

# Enable keyring authentication
export UV_KEYRING_PROVIDER=subprocess

# Set the username for the index
export UV_INDEX_PRIVATE_REGISTRY_USERNAME=oauth2accesstoken
```

!!! note

    Use the [`tool.uv.keyring-provider`](../../reference/settings.md#keyring-provider)
    setting to enable keyring in your `uv.toml` or `pyproject.toml`.

    You can also add the index username directly to the index URL.

## Publishing packages

Use `uv publish` to publish your own packages to Google Artifact Registry. See the
[Building and publishing guide](../package.md).

First, add a `publish-url` to the index that receives your packages:

```toml title="pyproject.toml" hl_lines="4"
[[tool.uv.index]]
name = "private-registry"
url = "https://<REGION>-python.pkg.dev/<PROJECT>/<REPOSITORY>/simple/"
publish-url = "https://<REGION>-python.pkg.dev/<PROJECT>/<REPOSITORY>/"
```

If you do not use keyring, configure the credentials:

```console
$ export UV_PUBLISH_USERNAME=oauth2accesstoken
$ export UV_PUBLISH_PASSWORD="$ARTIFACT_REGISTRY_TOKEN"
```

Publish the package:

```console
$ uv publish --index private-registry
```

If your project does not specify a `publish-url`, set `UV_PUBLISH_URL`:

```console
$ export UV_PUBLISH_URL=https://<REGION>-python.pkg.dev/<PROJECT>/<REPOSITORY>/
$ uv publish
```

This method is not recommended. Without an associated package index URL, uv cannot check whether the
package is already published before it uploads artifacts.
