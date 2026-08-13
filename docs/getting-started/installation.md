# Installing uv

## Installation methods

Install uv with a standalone installer or your preferred package manager.

### Standalone installer

Use the standalone installer to download and install uv:

=== "macOS and Linux"

    Use `curl` to download the installation script. Run the script with `sh`:

    ```console
    $ curl -LsSf https://astral.sh/uv/install.sh | sh
    ```

    If `curl` is not available, use `wget`:

    ```console
    $ wget -qO- https://astral.sh/uv/install.sh | sh
    ```

    To install a specific version, include that version in the URL:

    ```console
    $ curl -LsSf https://astral.sh/uv/0.12.10/install.sh | sh
    ```

=== "Windows"

    Use `irm` to download the installation script. Run the script with `iex`:

    ```pwsh-session
    PS> powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"
    ```

    Changing the [execution policy](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_execution_policies?view=powershell-7.4#powershell-execution-policies) permits the installation script to run.

    To install a specific version, include that version in the URL:

    ```pwsh-session
    PS> powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/0.12.10/install.ps1 | iex"
    ```

!!! tip

    Inspect the installation script before you run it:

    === "macOS and Linux"

        ```console
        $ curl -LsSf https://astral.sh/uv/install.sh | less
        ```

    === "Windows"

        ```pwsh-session
        PS> powershell -c "irm https://astral.sh/uv/install.ps1 | more"
        ```

    You can also download the installer or binaries directly from [GitHub](#github-releases).

Read the [installer reference](../reference/installer.md) to customize your uv installation.

### PyPI

uv is available on [PyPI](https://pypi.org/project/uv/).

Use `pipx` to install uv from PyPI in an isolated environment:

```console
$ pipx install uv
```

You can also install uv with `pip`:

```console
$ pip install uv
```

!!! note

    uv provides prebuilt distributions, also called wheels, for many platforms. If your platform has
    no wheel, the package manager builds uv from source. Building uv from source requires a Rust
    toolchain. Read the
    [contributing setup guide](https://github.com/astral-sh/uv/blob/main/CONTRIBUTING.md#setup)
    for details.

### Homebrew

Install uv from the core Homebrew packages:

```console
$ brew install uv
```

### MacPorts

Install uv with [MacPorts](https://ports.macports.org/port/uv/):

```console
$ sudo port install uv
```

### WinGet

Install uv with [WinGet](https://winstall.app/apps/astral-sh.uv):

```console
$ winget install --id=astral-sh.uv  -e
```

### Scoop

Install uv with [Scoop](https://scoop.sh/#/apps?q=uv):

```console
$ scoop install main/uv
```

### Docker

uv provides a Docker image at
[`ghcr.io/astral-sh/uv`](https://github.com/astral-sh/uv/pkgs/container/uv).

Read the guide on [using uv in Docker](../guides/integration/docker.md) for details.

### GitHub Releases

Download uv release artifacts directly from
[GitHub Releases](https://github.com/astral-sh/uv/releases).

Each release includes binaries for every supported platform. The release page also explains how to
run the standalone installer from `github.com` instead of `astral.sh`.

### Cargo

Install uv from [crates.io](https://crates.io):

```console
$ cargo install --locked uv
```

!!! note

    This command builds uv from source and requires a compatible Rust toolchain.

## Upgrading uv

If you used the standalone installer, update uv with this command:

```console
$ uv self update
```

!!! tip

    An update runs the installer again and can modify your shell profiles. Set `UV_NO_MODIFY_PATH=1`
    to prevent these changes.

If you used another installation method, uv disables self-updates. Use your package manager to
upgrade uv instead. For example, use `pip`:

```console
$ pip install --upgrade uv
```

## Shell autocompletion

!!! tip

    Run `echo $SHELL` to identify your shell.

To enable shell autocompletion for uv commands, run the command for your shell:

=== "Bash"

    ```bash
    echo 'eval "$(uv generate-shell-completion bash)"' >> ~/.bashrc
    ```

=== "Zsh"

    ```bash
    echo 'eval "$(uv generate-shell-completion zsh)"' >> ~/.zshrc
    ```

=== "fish"

    ```bash
    echo 'uv generate-shell-completion fish | source' > ~/.config/fish/completions/uv.fish
    ```

=== "Elvish"

    ```bash
    echo 'eval (uv generate-shell-completion elvish | slurp)' >> ~/.elvish/rc.elv
    ```

=== "PowerShell / pwsh"

    ```powershell
    if (!(Test-Path -Path $PROFILE)) {
      New-Item -ItemType File -Path $PROFILE -Force
    }
    Add-Content -Path $PROFILE -Value '(& uv generate-shell-completion powershell) | Out-String | Invoke-Expression'
    ```

To enable shell autocompletion for `uvx`, run the command for your shell:

=== "Bash"

    ```bash
    echo 'eval "$(uvx --generate-shell-completion bash)"' >> ~/.bashrc
    ```

=== "Zsh"

    ```bash
    echo 'eval "$(uvx --generate-shell-completion zsh)"' >> ~/.zshrc
    ```

=== "fish"

    ```bash
    echo 'uvx --generate-shell-completion fish | source' > ~/.config/fish/completions/uvx.fish
    ```

=== "Elvish"

    ```bash
    echo 'eval (uvx --generate-shell-completion elvish | slurp)' >> ~/.elvish/rc.elv
    ```

=== "PowerShell / pwsh"

    ```powershell
    if (!(Test-Path -Path $PROFILE)) {
      New-Item -ItemType File -Path $PROFILE -Force
    }
    Add-Content -Path $PROFILE -Value '(& uvx --generate-shell-completion powershell) | Out-String | Invoke-Expression'
    ```

Then, restart the shell or source its configuration file.

## Uninstallation

To remove uv from your system:

1.  Optionally, remove stored data:

    ```console
    $ uv cache clean
    $ rm -r "$(uv python dir)"
    $ rm -r "$(uv tool dir)"
    ```

    !!! tip

        You can remove stored uv data before you remove the binaries. The
        [storage reference](../reference/storage.md) lists where uv stores data.

2.  Remove the uv, uvx, and uvw binaries:

    === "macOS and Linux"

        ```console
        $ rm ~/.local/bin/uv ~/.local/bin/uvx
        ```

    === "Windows"

        ```pwsh-session
        PS> rm $HOME\.local\bin\uv.exe
        PS> rm $HOME\.local\bin\uvx.exe
        PS> rm $HOME\.local\bin\uvw.exe
        ```

    !!! note

        Versions earlier than 0.5.0 installed uv in `~/.cargo/bin`. Remove the binaries from that
        directory to uninstall those versions. Upgrading does not automatically remove old binaries
        from `~/.cargo/bin`.

## Next steps

Follow the [first steps](./first-steps.md) or read the [guides](../guides/index.md) to start using
uv.
