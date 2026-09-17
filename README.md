# uv upgrade cannot authenticate while uv sync --upgrade does

Issue: astral-sh/uv#21773

Classification: bug

## Summary

The reported command-specific authentication failure is reproducible. With a dependency pinned to
an explicit named index and Basic-auth credentials supplied by a matching `UV_EXTRA_INDEX_URL`,
`uv sync --upgrade` and `uv lock --upgrade` authenticate successfully, while project `uv upgrade`
queries the named index without authentication and fails with 401 followed by an unsatisfiable
resolution.

The reproduction used installed uv 0.12.13, so the behavior reported for uv 0.12.11 remains present
in a newer release. The report used macOS arm64 and Python 3.14.3; the reproduction used Linux
x86_64 and Python 3.12.3, showing that the failure is not limited to the reported platform or Python
version.

## Reproduction

Outcome: **reproducible**.

An isolated fixture under `/tmp` used a local HTTP Simple API server that required Basic
authentication. The server logged only the request path and whether an `Authorization` header was
present; it did not log header values. The index exposed a minimal
`my-private-pkg==1.0.0` wheel. All uv state, virtual environments, and command-specific caches were
kept inside the fixture directory.

The project configuration was:

```toml
[project]
name = "repro"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["my-private-pkg"]

[tool.uv]
package = false

[tool.uv.sources]
my-private-pkg = { index = "private" }

[[tool.uv.index]]
name = "private"
url = "http://127.0.0.1:<port>/simple/"
explicit = true
```

Each command used a separate cache and the same credential-bearing URL, with credential values
represented here by placeholders:

```console
$ UV_EXTRA_INDEX_URL='http://<username>:<password>@127.0.0.1:<port>/simple/' uv sync --upgrade
Resolved 2 packages
Prepared 1 package
Installed 1 package
 + my-private-pkg==1.0.0

$ UV_EXTRA_INDEX_URL='http://<username>:<password>@127.0.0.1:<port>/simple/' uv lock --upgrade
Resolved 2 packages

$ UV_EXTRA_INDEX_URL='http://<username>:<password>@127.0.0.1:<port>/simple/' uv upgrade
  × No solution found when resolving dependencies:
  ╰─▶ Because my-private-pkg was not found in the package registry and your
      project depends on my-private-pkg, we can conclude that your project's
      requirements are unsatisfiable.

hint: An index URL (http://127.0.0.1:<port>/simple/) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)
```

The first two commands exited 0 and the server observed authenticated index requests. `uv upgrade`
exited 1, and its Simple API request was observed without authentication. In particular,
`uv upgrade` used a fresh cache, ruling out metadata cached by either successful control command.

Environment:

- uv 0.12.13 (`x86_64-unknown-linux-gnu`), installed executable on `PATH`
- Linux 6.17.0-1022-azure x86_64
- CPython 3.12.3 from `/usr/bin/python3`
- Python downloads disabled and home, XDG, Python-install, virtual-environment, and cache paths
  isolated to the temporary fixture

Existing coverage does not exercise this failing combination. The integration test
`crates/uv/tests/it/upgrade.rs::upgrade_allows_registry_source` verifies that project `uv upgrade`
can resolve from an explicit public registry source, but it does not require authentication or
supply credentials through `UV_EXTRA_INDEX_URL`. Authenticated-index tests exist for other command
paths, including `crates/uv/tests/lock/lock.rs::lock_index_workspace_member`, but they do not cover
project `uv upgrade`.

## Classification

This is a bug because otherwise equivalent project-resolution commands behave differently with the
same project configuration and authentication input. The control commands authenticated and
resolved successfully, while `uv upgrade` omitted authentication and failed before it could update
the dependency declaration.

Current command wiring is consistent with the observation: `UpgradeArgs` has only package and
exclusion fields, `uv upgrade --help` exposes no index or registry-client options, and
`UpgradeSettings` begins with default resolver CLI options before combining filesystem and global
environment settings. In contrast, resolving commands such as `uv lock` include the shared
resolver arguments that parse `UV_EXTRA_INDEX_URL`. This is supporting implementation evidence;
the reproduction itself confirms the command-specific request behavior without relying on a
source-only root-cause inference.

Named-index credential variables (`UV_INDEX_PRIVATE_USERNAME` and
`UV_INDEX_PRIVATE_PASSWORD`) were not part of the reported failing input and were not evaluated in
this reproduction, so no workaround is claimed from that separate credential path.

## Related

- astral-sh/uv#19678 (merged pull request), “Add initial hidden `uv upgrade` command” — introduced
  the project-upgrade resolver path and its dedicated CLI, settings, implementation, and integration
  tests. It is relevant implementation history but did not discuss authentication.
- astral-sh/uv#14806 (closed issue), “uv tool upgrade does not authenticate against GitLab private
  pypi package registry” — reports a similar symptom for `uv tool upgrade`, whose persisted receipt
  and index-merging path is separate from project `uv upgrade`.
- astral-sh/uv#14858 (merged pull request), “Respect credentials from all defined indexes” — fixed
  astral-sh/uv#14806 for the tool-upgrade path and does not cover the project-upgrade command used
  here.

No related issue or pull request listed above already fixes or tests the reproduced project
`uv upgrade` behavior.
