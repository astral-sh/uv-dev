# Having a git credentials helper configured per-project (not globally) for `uv` to use

Issue: astral-sh/uv#21287

Classification: duplicate

## Summary

The reporter can install private Git dependencies after configuring a Git credential helper
globally, but wants the helper configuration scoped to the project so it cannot affect unrelated
projects on the same machine. A helper configured in the invoking repository's local `.git/config`
is not selected for uv's fetch because uv runs Git from its own cache repository.

astral-sh/uv#8441 is the closest and canonical existing report. It describes the same loss of the
invoking repository's Git configuration context, including a credential helper selected by
location, and remains open with `bug` and `needs-design` labels. A repository member has confirmed
that astral-sh/uv#21287 shares the root cause of astral-sh/uv#8441. astral-sh/uv#20964 is adjacent:
it adds coverage for credential-helper authentication configured under a temporary home directory,
but does not cover or implement caller-project-local Git configuration.

## Draft response

This is not currently possible through the invoking repository's local `.git/config`. uv performs
the dependency fetch from its cached Git repository, so Git does not select the local configuration
from the project where uv was invoked.

The same underlying limitation is already tracked in astral-sh/uv#8441, including project- or
directory-specific credential-helper configuration, so let's centralize the discussion there. As a
command-scoped workaround, you can point `GIT_CONFIG_GLOBAL` at a separate project-specific Git
configuration file when invoking uv; uv inherits that Git setting, while other projects and normal
Git invocations remain unaffected.

## Classification

This is a duplicate of astral-sh/uv#8441. Although astral-sh/uv#21287 is phrased as a question and
uses a repository-local `.git/config` rather than an `includeIf`-selected file, both reports ask uv
to retain the invoking project's Git configuration context when fetching a private dependency.
The implementation runs `git fetch` with uv's cache repository as Git's working directory, which
explains why the caller repository's local configuration is not selected. The narrower reproduction
does not require a separate discussion while astral-sh/uv#8441 remains open. This relationship is
now confirmed by a repository member in the discussion on astral-sh/uv#21287.

This is not a duplicate of the broader astral-sh/uv#8529 request for first-class Git authentication
configuration in `[tool.uv.sources]`, nor of astral-sh/uv#2048 and astral-sh/uv#20964, which concern
test coverage for an already configured home/global credential helper.

## Related

- astral-sh/uv#8441 — Open issue, “Trying to use custom gitconfig location, appears to get ignored.”
  This is the canonical match: it reports that credential-helper and SSH settings selected from the
  invoking project's Git context are missed when uv performs Git operations in its cache. Maintainer
  discussion confirms that preserving the original working-directory context needs design.
- astral-sh/uv#20964 — Open pull request, “Test Git credential helper authentication.” This adjacent
  work verifies that uv can retrieve private Git dependency credentials from a helper configured in
  temporary home/global Git configuration. It does not test or implement repository-local Git
  configuration, which distinguishes basic helper support from the scope requested here.

## Search evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
included `credential helper`, `credential.helper`, `git config --global`, and `local git config`.
Conceptual queries covered per-project or project-scoped Git authentication, custom Git config,
private Git dependencies, and Git credential lookup. Fix-oriented review included the switch to the
Git CLI and later authentication documentation and credential-helper test work.

The strongest candidates and their reference chains were inspected. astral-sh/uv#8441 was followed
to astral-sh/uv#8413; the latter concerns Git's handling of a custom netrc location and was closed as
external, so it is not the canonical match. astral-sh/uv#8529 and astral-sh/uv#12368 concern broader
ways to supply Git authentication. astral-sh/uv#19419 and astral-sh/uv#19423 address a fixed,
workspace-member `[tool.uv.sources]` credential propagation bug under `uv sync --frozen`, not local
Git configuration. Merged astral-sh/uv#1781 and astral-sh/uv#13850 establish the Git CLI and global
credential-helper behavior but do not implement the requested project scope.
