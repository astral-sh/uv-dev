# The `uv auth` CLI

uv provides commands to store and retrieve credentials for services.

## Logging in to a service

To add credentials for a service, use `uv auth login`:

```console
$ uv auth login example.com
```

The command prompts for credentials.

Alternatively, use `--username` and `--password` to provide credentials. Use `--token` for services
that accept `__token__` or an arbitrary username.

!!! note

    Pass secrets through stdin. Use `-` to read a value from stdin, such as with `--password`:

    ```console
    $ echo 'my-password' | uv auth login example.com --password -
    ```

    Use the same pattern with `--token`.

After uv stores credentials, it uses them for package operations that fetch content from the
service. uv supports HTTPS Basic authentication only. It does not use these credentials for Git
requests.

!!! note

    uv does not validate the credentials when it stores them. Incorrect credentials do not cause
    the login command to fail.

## Logging out of a service

To remove credentials, use `uv auth logout`:

```console
$ uv auth logout example.com
```

!!! note

    This command removes credentials from local storage only. It does not invalidate them on the
    remote server.

## Showing credentials for a service

To show the credential for a URL, use `uv auth token`:

```console
$ uv auth token example.com
```

If the login included a username, include that username:

```console
$ uv auth token --username foo example.com
```

## Using credentials with external tools

`uv auth helper` lets external tools request HTTP credentials from uv. uv supports the
[Bazel credential helper protocol](https://github.com/bazelbuild/proposals/blob/main/designs/2022-06-07-bazel-credential-helpers.md).

External tools invoke this command. It reads a JSON request from stdin and writes a JSON response to
stdout. If matching credentials exist, the response includes the `Authorization` header:

```console
$ echo '{"uri": "https://example.com/path"}' | uv --preview-features auth-helper auth helper --protocol=bazel get
{"headers":{"Authorization":["Basic ..."]}}
```

If uv does not find credentials, it returns an empty set of headers:

```json
{ "headers": {} }
```

!!! note

    `uv auth helper` is experimental. Use `--preview-features auth-helper` or
    `UV_PREVIEW_FEATURES=auth-helper` to disable the warning.

The [Bazel integration guide](../../guides/integration/bazel.md) explains how to use this command
with Bazel.

## Configuring the storage backend

uv saves credentials in its [credentials store](./http.md#the-uv-credentials-store).

By default, uv saves credentials in a plaintext file. Set `UV_PREVIEW_FEATURES=native-auth` to use
encrypted system-native storage.
