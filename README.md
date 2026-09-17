# uv upgrade cannot authenticate while uv sync --upgrade does

Issue: astral-sh/uv#21773

Classification: bug

## Summary

The reporter configures a package to use an explicit named Azure private index and supplies Basic
authentication through `UV_EXTRA_INDEX_URL`. On uv 0.12.11, `uv sync --upgrade` and
`uv lock --upgrade` resolve successfully, but project `uv upgrade` queries the configured index
without the supplied credentials and reports a 401 followed by an unsatisfiable resolution.

No existing issue or pull request tracks this same project-command split. Current source supports
the report: project `uv upgrade` performs resolution but its CLI and settings path omits the shared
resolver/index arguments that parse `UV_EXTRA_INDEX_URL` and related index and authentication
options.

## Report details

- Symptom: only project `uv upgrade` fails authentication and reports the private package absent.
- Expected parity: `uv upgrade`, `uv sync --upgrade`, and `uv lock --upgrade` should use the same
  private-index configuration during resolution.
- Trigger: an explicit named index selected through `[tool.uv.sources]`, with credentials embedded
  in the otherwise matching `UV_EXTRA_INDEX_URL`.
- Exact diagnostic: the configured index cannot be queried because of invalid authentication
  credentials and returns `401 Unauthorized`.
- Additional concern: project `uv upgrade` exposes no index or registry-client flags. The reporter
  has not tested named-index username/password environment variables.

## Draft response

Thanks for the clear reproduction. I confirmed this is a bug in the current command wiring:
`uv upgrade` resolves packages but does not include the shared resolver/index argument group, so
`UV_EXTRA_INDEX_URL` and the corresponding CLI index and authentication options are not read on
this path. `uv sync` and `uv lock` do include that configuration.

Named-index credentials are looked up directly from the configured index, so
`UV_INDEX_PRIVATE_USERNAME` and `UV_INDEX_PRIVATE_PASSWORD` should be a temporary workaround. If
those also fail, please share redacted `-vv` output. The code needs to align `uv upgrade` with the
other project resolver commands and add authenticated explicit-index integration coverage.

## Classification

This is a bug because the current implementation creates a command-specific configuration gap for
a resolver-backed operation:

- `UpgradeArgs` contains package selection and exclusion only.
- `UV_EXTRA_INDEX_URL` and the index and registry-client flags are declared in the shared resolver
  argument group used by other resolving commands.
- `UpgradeSettings` starts from default resolver options and combines them with filesystem
  configuration. The named explicit index is therefore retained, but the URL carrying credentials
  is never parsed for this command.
- Named-index credentials follow a separate path: configured indexes retrieve
  `UV_INDEX_{name}_USERNAME` and `UV_INDEX_{name}_PASSWORD` directly. That makes the reporter's
  suggested named-index variables a source-supported workaround, not evidence that they fail too.
- Existing `uv upgrade` integration coverage includes an explicit registry source backed by a
  public index, but has no authenticated-index case.

No open issue or pull request already centralizes this exact project `uv upgrade` regression, so it
should not be classified as a duplicate.

## Related

- astral-sh/uv#19678 (merged pull request), “Add initial hidden `uv upgrade` command” — introduced
  the exact project-upgrade resolver path and touched its CLI, settings, implementation, and
  integration tests. Current code still reflects its dedicated minimal argument wiring. It is
  relevant implementation history, but it did not discuss authentication and is not itself a fix
  for this newly reported behavior.
- astral-sh/uv#14806 (closed issue), “uv tool upgrade does not authenticate against GitLab private
  pypi package registry” — the closest earlier observable symptom: an upgrade command lost
  private-index authentication while other usage worked. It applies to `uv tool upgrade`, whose
  persisted receipt and index-merging path is separate from project `uv upgrade`.
- astral-sh/uv#14858 (merged pull request), “Respect credentials from all defined indexes” — fixed
  astral-sh/uv#14806 by considering credential-bearing configured indexes alongside the tool
  receipt. That mechanism explains why the prior report is adjacent rather than a duplicate and
  does not address the missing project-upgrade resolver arguments.

## Search and supporting evidence

Literal searches covered `uv upgrade`, `UV_EXTRA_INDEX_URL`, `extra-index-url`, the exact lack-of-
credentials hint, `401 Unauthorized`, private indexes, explicit indexes, and Azure authentication.
Conceptual searches covered upgrade/authentication parity, credentials ignored by one command,
named-index credentials, registry-client and index options, and historical authentication fixes.
The search included open and closed issues and open, closed, and merged pull requests. The strongest
candidates, maintainer comments, referenced fixes, and the project-upgrade implementation history
were inspected.

Several plausible candidates were ruled out:

- astral-sh/uv#16478 concerns the inability to attach `authenticate = "always"` to an index supplied
  through CLI or pip-style URL options. Here, the index is already a named filesystem-configured
  index and project `uv upgrade` does not parse `UV_EXTRA_INDEX_URL` at all.
- astral-sh/uv#12611 and its merged fix astral-sh/uv#12631 concern explicit indexes being omitted
  from authentication-policy construction, a later stage than the configuration omission here.
- astral-sh/uv#13216 concerns uv 0.7 index-security behavior rather than a project-upgrade-only path.
- astral-sh/uv#19817 concerns named credential environment variables with `uv sync`, the command
  that succeeds in this report.
- astral-sh/uv#21216 concerns username persistence in `uv tool` receipts, not project resolution.

