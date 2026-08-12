# The `uv auth` CLI

uv provides a high-level interface for storing and retrieving credentials from services.

## Logging in to a service

To add credentials for service, use the `uv auth login` command:

```console
$ uv auth login example.com
```

This will prompt for the credentials.

The credentials can also be provided using the `--username` and `--password` options, or the
`--token` option for services which use a `__token__` or arbitrary username.

!!! note

    We recommend providing the secret via stdin. Use `-` to indicate the value should be read from
    stdin, e.g., for `--password`:

    ```console
    $ echo 'my-password' | uv auth login example.com --password -
    ```

    The same pattern can be used with `--token`.

Once credentials are added, uv will use them for packaging operations that require fetching content
from the given service. At this time, only HTTPS Basic authentication is supported. The credentials
will not yet be used for Git requests.

!!! note

    The credentials will not be validated, i.e., incorrect credentials will not fail.

## Logging in with OIDC device authorization

For indexes that support OpenID Connect, `uv auth login` can authenticate using the OAuth 2.0 Device
Authorization Grant ([RFC 8628](https://datatracker.ietf.org/doc/html/rfc8628)). When no
`--username`, `--password`, or `--token` is provided, uv probes
`<url>/.well-known/openid-configuration` and, if device authorization endpoints are advertised, runs
the device flow:

```console
$ uv auth login https://my-registry.example.com
Open https://my-registry.example.com/device and enter code: WDJB-MJHT
```

uv attempts to open the verification URL in your browser; if that fails, you can open the URL
manually and enter the code. Once authorization completes, the resulting bearer token is stored in
the credential store and used for subsequent requests to that origin. If the authorization server
provides an expiration time, uv records it and stops using the token shortly before it expires. Run
`uv auth login` again to obtain a new token.

For indexes whose OIDC provider lives on a different domain than the package index (e.g., Azure
DevOps Artifacts uses Microsoft Entra), the issuer URL, client ID, and scope can be supplied via
flags or under [`[tool.uv.index.oidc]`](../indexes.md#authenticating-via-oidc-device-authorization).
For example, with Azure Artifacts:

```console
$ uv auth login https://pkgs.dev.azure.com \
    --issuer https://login.microsoftonline.com/<TENANT_ID>/v2.0 \
    --client-id d5a56ea4-7369-46b8-a538-c370805301bf \
    --scope 499b84ac-1321-427f-aa17-267ca6975798/.default
```

CLI flags take precedence over any matching `[[tool.uv.index]]` configuration. If `--issuer` is
provided but discovery fails, uv reports an error rather than falling back to the username/password
prompt.

## Logging out of a service

To remove credentials, use the `uv auth logout` command:

```console
$ uv auth logout example.com
```

!!! note

    The credentials will not be invalidated with the remote server, i.e., they will only be removed
    from local storage not rendered unusable.

## Showing credentials for a service

To show the credential stored for a given URL, use the `uv auth token` command:

```console
$ uv auth token example.com
```

If a username was used to log in, it will need to be provided as well, e.g.:

```console
$ uv auth token --username foo example.com
```

## Using credentials with external tools

`uv auth helper` allows tools that support credential helpers to request HTTP credentials from uv.
At this time, uv supports the
[Bazel credential helper protocol](https://github.com/bazelbuild/proposals/blob/main/designs/2022-06-07-bazel-credential-helpers.md).

The command is intended to be invoked by external tools. It reads a JSON request from stdin and
writes a JSON response to stdout. When matching credentials are available, the response includes the
`Authorization` header:

```console
$ echo '{"uri": "https://example.com/path"}' | uv --preview-features auth-helper auth helper --protocol=bazel get
{"headers":{"Authorization":["Basic ..."]}}
```

If no credentials are found, uv will return an empty set of headers:

```json
{ "headers": {} }
```

!!! note

    `uv auth helper` is experimental. Use `--preview-features auth-helper` or
    `UV_PREVIEW_FEATURES=auth-helper` to disable the warning.

The [Bazel integration guide](../../guides/integration/bazel.md) explains how to use this command
with Bazel.

## Configuring the storage backend

Credentials are persisted to the uv [credentials store](./http.md#the-uv-credentials-store).

By default, credentials are written to a plaintext file. An encrypted system-native storage backend
can be enabled with `UV_PREVIEW_FEATURES=native-auth`.
