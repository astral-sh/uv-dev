---
title: AWS CodeArtifact
description: Install and publish Python packages with uv and AWS CodeArtifact.
---

# AWS CodeArtifact

Use an access token or the [`keyring`](https://github.com/jaraco/keyring) package to install
packages from
[AWS CodeArtifact](https://docs.aws.amazon.com/codeartifact/latest/ug/using-python.html).

!!! note

    This guide requires an installed and authenticated [`awscli`](https://aws.amazon.com/cli/).

Add the index to your project:

```toml title="pyproject.toml"
[[tool.uv.index]]
name = "private-registry"
url = "https://<DOMAIN>-<ACCOUNT_ID>.d.codeartifact.<REGION>.amazonaws.com/pypi/<REPOSITORY>/simple/"
```

## Authenticate with an AWS access token

Use the "Basic" HTTP authentication scheme. Put the access token in the password field of the URL.
Set the username to `aws`, or authentication fails.

Generate a token with `awscli`:

```bash
export AWS_CODEARTIFACT_TOKEN="$(
    aws codeartifact get-authorization-token \
    --domain <DOMAIN> \
    --domain-owner <ACCOUNT_ID> \
    --query authorizationToken \
    --output text
)"
```

!!! note

    You might need to pass extra parameters, such as `--region`, to generate the token. This command
    is a basic example.

Set the index credentials:

```bash
export UV_INDEX_PRIVATE_REGISTRY_USERNAME=aws
export UV_INDEX_PRIVATE_REGISTRY_PASSWORD="$AWS_CODEARTIFACT_TOKEN"
```

!!! note

    Make sure `PRIVATE_REGISTRY` matches the index name in your `pyproject.toml`.

## Authenticate with `keyring` and `keyrings.codeartifact`

To authenticate to Artifact Registry, use the [`keyring`](https://github.com/jaraco/keyring) package
with the [`keyrings.codeartifact` plugin](https://github.com/jmkeyes/keyrings.codeartifact). Install
both packages from a source other than Artifact Registry because authentication requires them.

The `keyrings.codeartifact` plugin wraps [boto3](https://pypi.org/project/boto3/) to generate
short-lived access tokens. It securely stores the tokens in the system keyring and refreshes expired
tokens.

uv only supports the `keyring` package in
[subprocess mode](../../reference/settings.md#keyring-provider). The `keyring` executable must be in
the `PATH`. Install it globally or in the active environment. The `keyring` CLI requires the
username `aws` in the URL.

```bash
# Pre-install keyring and AWS CodeArtifact plugin from the public PyPI
uv tool install keyring --with keyrings.codeartifact

# Enable keyring authentication
export UV_KEYRING_PROVIDER=subprocess

# Set the username for the index
export UV_INDEX_PRIVATE_REGISTRY_USERNAME=aws
```

!!! note

    Use the [`tool.uv.keyring-provider`](../../reference/settings.md#keyring-provider)
    setting to enable keyring in your `uv.toml` or `pyproject.toml`.

    You can also add the index username directly to the index URL.

## Publishing packages

Use `uv publish` to publish your own packages to AWS CodeArtifact. See the
[Building and publishing guide](../package.md).

First, add a `publish-url` to the index that receives your packages:

```toml title="pyproject.toml" hl_lines="4"
[[tool.uv.index]]
name = "private-registry"
url = "https://<DOMAIN>-<ACCOUNT_ID>.d.codeartifact.<REGION>.amazonaws.com/pypi/<REPOSITORY>/simple/"
publish-url = "https://<DOMAIN>-<ACCOUNT_ID>.d.codeartifact.<REGION>.amazonaws.com/pypi/<REPOSITORY>/"
```

If you do not use keyring, configure the credentials:

```console
$ export UV_PUBLISH_USERNAME=aws
$ export UV_PUBLISH_PASSWORD="$AWS_CODEARTIFACT_TOKEN"
```

Publish the package:

```console
$ uv publish --index private-registry
```

If your project does not specify a `publish-url`, set `UV_PUBLISH_URL`:

```console
$ export UV_PUBLISH_URL=https://<DOMAIN>-<ACCOUNT_ID>.d.codeartifact.<REGION>.amazonaws.com/pypi/<REPOSITORY>/
$ uv publish
```

This method is not recommended. Without an associated package index URL, uv cannot check whether the
package is already published before it uploads artifacts.
