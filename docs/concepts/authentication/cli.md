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

## Logging out of a service

To remove credentials, use the `uv auth logout` command:

```console
$ uv auth logout example.com
```

When using the system-native storage backend, uv can infer the username if exactly one account
matches the service. If multiple accounts match, specify `--username`.

!!! note

    The credentials will not be invalidated with the remote server, i.e., they will only be removed
    from local storage not rendered unusable.

## Showing credentials for a service

To show the credential stored for a given URL, use the `uv auth token` command:

```console
$ uv auth token example.com
```

With the default plaintext storage backend, a username used to log in must also be provided, e.g.:

```console
$ uv auth token --username foo example.com
```

When using the system-native storage backend, uv can infer the username if exactly one account
matches the service. If multiple accounts match, specify `--username`.

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
is available in [preview](../preview.md) and can be enabled with `--preview-features native-auth` or
`UV_PREVIEW_FEATURES=native-auth`.

When native storage is enabled, existing plaintext credentials are not used. uv warns if a plaintext
credentials file exists; run `uv auth login` again to add credentials to the native store.
