# The uv installer

## Changing the installation path

By default, the installer places uv in the user
[executable directory](./storage.md#executable-directory).

The `UV_INSTALL_DIR` environment variable changes the installation path:

=== "macOS and Linux"

    ```console
    $ curl -LsSf https://astral.sh/uv/install.sh | env UV_INSTALL_DIR="/custom/path" sh
    ```

=== "Windows"

    ```pwsh-session
    PS> powershell -ExecutionPolicy ByPass -c {$env:UV_INSTALL_DIR = "C:\Custom\Path";irm https://astral.sh/uv/install.ps1 | iex}
    ```

!!! note

    Changing the installation path only changes the location of the uv executable. uv still stores
    its cache, Python installations, tools, and other data in their default locations. The
    [storage reference](./storage.md) describes these locations and their settings.

## Disabling shell modifications

The installer may update shell profiles to add the uv executable to `PATH`. The `UV_NO_MODIFY_PATH`
environment variable disables these updates:

```console
$ curl -LsSf https://astral.sh/uv/install.sh | env UV_NO_MODIFY_PATH=1 sh
```

After installation with `UV_NO_MODIFY_PATH`, commands such as `uv self update` do not change shell
profiles.

## Unmanaged installations

In temporary environments such as CI, `UV_UNMANAGED_INSTALL` installs uv at a specific path. It also
prevents changes to shell profiles and environment variables:

```console
$ curl -LsSf https://astral.sh/uv/install.sh | env UV_UNMANAGED_INSTALL="/custom/path" sh
```

The `UV_UNMANAGED_INSTALL` setting also disables self-updates with `uv self update`.

## Passing options to the installation script

Environment variables are the recommended approach because they work consistently across platforms.
The installation script also accepts options directly. For example, this command shows the available
options:

```console
$ curl -LsSf https://astral.sh/uv/install.sh | sh -s -- --help
```
