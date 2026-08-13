---
title: Azure Artifacts
description: Install and publish Python packages with uv and Azure Artifacts.
---

# Azure Artifacts

Use a
[Personal Access Token](https://learn.microsoft.com/en-us/azure/devops/organizations/accounts/use-personal-access-tokens-to-authenticate?view=azure-devops&tabs=Windows)
(PAT) or the [`keyring`](https://github.com/jaraco/keyring) package to install packages from
[Azure Artifacts](https://learn.microsoft.com/en-us/azure/devops/artifacts/start-using-azure-artifacts?view=azure-devops&tabs=nuget%2Cnugetserver).

To use Azure Artifacts, add the index to your project:

```toml title="pyproject.toml"
[[tool.uv.index]]
name = "private-registry"
url = "https://pkgs.dev.azure.com/<ORGANIZATION>/<PROJECT>/_packaging/<FEED>/pypi/simple/"
```

## Authenticate with an Azure access token

If you have a personal access token (PAT), use the "Basic" HTTP authentication scheme. For example,
use
[`$(System.AccessToken)` in an Azure pipeline](https://learn.microsoft.com/en-us/azure/devops/pipelines/build/variables?view=azure-devops&tabs=yaml#systemaccesstoken).
Put the PAT in the password field of the URL. Include a username, which can be any string.

If the token is in the `$AZURE_ARTIFACTS_TOKEN` environment variable, set the index credentials:

```bash
export UV_INDEX_PRIVATE_REGISTRY_USERNAME=dummy
export UV_INDEX_PRIVATE_REGISTRY_PASSWORD="$AZURE_ARTIFACTS_TOKEN"
```

!!! note

    Make sure `PRIVATE_REGISTRY` matches the index name in your `pyproject.toml`.

## Authenticate with `keyring` and `artifacts-keyring`

To authenticate to Azure Artifacts, use the [`keyring`](https://github.com/jaraco/keyring) package
with the [`artifacts-keyring` plugin](https://github.com/Microsoft/artifacts-keyring). Install both
packages from a source other than Azure Artifacts because authentication requires them.

The `artifacts-keyring` plugin wraps the
[Azure Artifacts Credential Provider tool](https://github.com/microsoft/artifacts-credprovider). The
credential provider supports several authentication modes, including interactive login. See the
[tool's documentation](https://github.com/microsoft/artifacts-credprovider) for configuration
details.

uv only supports the `keyring` package in
[subprocess mode](../../reference/settings.md#keyring-provider). The `keyring` executable must be in
the `PATH`. Install it globally or in the active environment. The `keyring` CLI requires the
username `VssSessionToken` in the URL.

```bash
# Pre-install keyring and the Artifacts plugin from the public PyPI
uv tool install keyring --with artifacts-keyring

# Enable keyring authentication
export UV_KEYRING_PROVIDER=subprocess

# Set the username for the index
export UV_INDEX_PRIVATE_REGISTRY_USERNAME=VssSessionToken
```

!!! note

    Use the [`tool.uv.keyring-provider`](../../reference/settings.md#keyring-provider)
    setting to enable keyring in your `uv.toml` or `pyproject.toml`.

    You can also add the index username directly to the index URL.

## Publishing packages

Use `uv publish` to publish your own packages to Azure Artifacts. See the
[Building and publishing guide](../package.md).

First, add a `publish-url` to the index that receives your packages:

```toml title="pyproject.toml" hl_lines="4"
[[tool.uv.index]]
name = "private-registry"
url = "https://pkgs.dev.azure.com/<ORGANIZATION>/<PROJECT>/_packaging/<FEED>/pypi/simple/"
publish-url = "https://pkgs.dev.azure.com/<ORGANIZATION>/<PROJECT>/_packaging/<FEED>/pypi/upload/"
```

If you do not use keyring, configure the credentials:

```console
$ export UV_PUBLISH_USERNAME=dummy
$ export UV_PUBLISH_PASSWORD="$AZURE_ARTIFACTS_TOKEN"
```

Publish the package:

```console
$ uv publish --index private-registry
```

If your project does not specify a `publish-url`, set `UV_PUBLISH_URL`:

```console
$ export UV_PUBLISH_URL=https://pkgs.dev.azure.com/<ORGANIZATION>/<PROJECT>/_packaging/<FEED>/pypi/upload/
$ uv publish
```

This method is not recommended. Without an associated package index URL, uv cannot check whether the
package is already published before it uploads artifacts.
