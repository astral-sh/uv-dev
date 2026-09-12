# Git credentials

uv installs packages from private Git repositories with SSH or HTTP authentication.

## SSH authentication

To authenticate using an SSH key, use the `ssh://` protocol:

- `git+ssh://git@<hostname>/...` (e.g., `git+ssh://git@github.com/astral-sh/uv`)
- `git+ssh://git@<host>/...` (e.g., `git+ssh://git@github.com-key-2/astral-sh/uv`)

SSH authentication requires the username `git`.

See the
[GitHub SSH documentation](https://docs.github.com/en/authentication/connecting-to-github-with-ssh/about-ssh)
for SSH configuration instructions.

### HTTP authentication

To use HTTP Basic authentication with a password or token:

- `git+https://<user>:<token>@<hostname>/...` (e.g.,
  `git+https://git:github_pat_asdf@github.com/astral-sh/uv`)
- `git+https://<token>@<hostname>/...` (e.g., `git+https://github_pat_asdf@github.com/astral-sh/uv`)
- `git+https://<user>@<hostname>/...` (e.g., `git+https://git@github.com/astral-sh/uv`)

!!! note

    A GitHub personal access token accepts any username. GitHub does not accept an account name and
    password in these URLs, although other hosts might.

If a URL does not include required credentials, uv queries the
[Git credential helper](#git-credential-helpers).

## Persistence of credentials

`uv add` _does not_ save Git credentials in `pyproject.toml` or `uv.lock`. These files often appear
in source control and distributions. Credentials in these files can therefore become public.

A configured Git credential helper can save credentials for later requests. Without a credential
helper or existing credentials, uv cannot fetch private dependencies.

The `--raw` option for `uv add` can force uv to save Git credentials. Use a
[credential helper](#git-credential-helpers) instead to avoid exposing them.

## Git credential helpers

Git credential helpers store and retrieve Git credentials. See the
[Git documentation](https://git-scm.com/doc/credential-helpers) to learn more.

For GitHub, [install the `gh` CLI](https://github.com/cli/cli#installation), then run:

```console
$ gh auth login
```

See the [`gh auth login`](https://cli.github.com/manual/gh_auth_login) documentation for more
details.

!!! note

    Interactive `gh auth login` configures the credential helper automatically. However,
    `gh auth login --with-token` does not. After a token-based login, run
    [`gh auth setup-git`](https://cli.github.com/manual/gh_auth_setup-git) to configure the helper.
    The [GitHub Actions guide](../../guides/integration/github.md#private-repos) shows this workflow.
