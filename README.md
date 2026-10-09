# Adding Private Git Repo With SSH Drops Username

Issue: astral-sh/uv#22277

Classification: duplicate

## Summary

The reporter runs `uv add 'some_package@git+ssh://mygituser@my-gitserver/myorg/some-package.git'` and gets a `tool.uv.sources` entry whose Git URL omits `mygituser@`. They expect that username to remain so subsequent dependency updates can authenticate. The reported environment is uv 0.12.23, Debian 13.7 and Python 3.13.5.

In the issue comments, the reporter clarifies that the non-`git` SSH account is shared by all developers, analogous to GitHub's conventional `git` account, and suggests configurability. A maintainer notes that stripping is intentional and asks whether the account is personal. The source confirms that the exception depends on the literal username, not on the hostname.

astral-sh/uv#15034 is the closest existing discussion. astral-sh/uv#6074 explains default credential stripping; astral-sh/uv#13795 and astral-sh/uv#13799 concern the narrower, previously fixed git export-masking problem.

## Draft response

`uv add` currently removes custom SSH usernames from saved Git sources; it preserves the literal username `git` regardless of hostname. Your shared SSH account hits the same username-persistence problem tracked in astral-sh/uv#15034, whose example uses HTTPS. Please add your SSH case there so we can track these together.

For now, `uv add --raw 'some_package@git+ssh://mygituser@my-gitserver/myorg/some-package.git'` retains the URL in `project.dependencies` instead of creating a `tool.uv.sources` entry.

## Classification

The open astral-sh/uv#15034 already tracks uv add discarding a required Git username when writing tool.uv.sources. Source inspection confirms that HTTPS and custom SSH usernames reach the same removal logic; the shared SSH account is an additional triggering condition. This is an underlying correctness problem despite intentional credential stripping and the current question label, but duplicate takes precedence. The historical astral-sh/uv#13799 fix covered only git and does not establish a regression for custom usernames.

The existing issue's HTTPS/.netrc example and this SSH example both request retention of a username needed to reuse a saved Git source. Their authentication transports differ, but the command, persisted field, destructive transformation and requested correction match. astral-sh/uv#15034 remains open, carries the bug label, and has no comments or cross-references in the inspected timeline. Discussion can be centralized there.

The documentation's restriction to the username `git` explains the current limitation; it does not make removal of a required shared SSH account correct. There is no evidence that this custom-account case was fixed and then regressed. No runtime reproduction against the reporter's private server was attempted.

## Related

- astral-sh/uv#15034 (issue, open) — uv add with authentication appropriately doesn't store the secret, but should store the username. Tracks the same uv add behavior: a required username is removed from the Git URL saved in tool.uv.sources, preventing later authentication. Its reproduction uses HTTPS and .netrc on uv 0.8.4; this report adds a shared, non-git SSH account. Both use the same source credential-removal path.
- astral-sh/uv#6074 (pull request, merged) — Redact Git credentials from `pyproject.toml`. Introduced default Git credential removal during uv add, closing astral-sh/uv#6056 on 2024-08-14. Its description explicitly preserves raw sources and uses an in-process store for the immediate lock operation. This establishes the persistence policy behind the symptom, rather than a fix for required SSH usernames.
- astral-sh/uv#13795 (issue, closed) — ssh username `git` obfuscated in 0.7.9. Historical SSH username corruption in uv export: uv 0.7.9 wrote **** instead of git into requirements.txt. Unlike the new report, it concerned export masking of the conventional git account, not removal of a custom username from pyproject.toml.
- astral-sh/uv#13799 (pull request, merged) — Don't redact username `git` for SSH. Merged 2025-06-03 to fix astral-sh/uv#13795 specifically for the literal username git. Its stated scope and the current username-only exception do not establish that arbitrary SSH usernames were previously supported, so the new report is not shown to regress this fix.

## Supporting evidence

Source inspected at checkout commit `01b62808962d7abfe2d10f43d652f357d8038202`:

- `crates/uv-project-commands/src/add.rs:952` explains why Git credentials are removed from files intended for version control. At line 970, the Git source passes through `Credentials::from_url`; line 976 calls `git.remove_credentials()` before saving the source.
- `crates/uv-auth/src/credentials.rs:232` treats a username-only URL as containing credentials. It returns no credentials only when both username and password are absent.
- `crates/uv-redacted/src/lib.rs:200` removes the username and password unless `is_ssh_git_username` accepts the URL. At line 306, that predicate requires the literal username `git`, no password, and an allowed scheme. It does not check the hostname. This directly explains why a custom account is dropped.
- `crates/uv-redacted/src/lib.rs:438` and line 479 contain existing tests for preserving the literal `git` username in SSH display and credential removal. These tests do not establish support for arbitrary SSH usernames.
- `docs/concepts/authentication/git.md` documents the `git` username restriction, default removal of Git credentials from project and lock files, and the `uv add --raw` escape hatch.
- `crates/uv-project-commands/src/add.rs:903` bypasses `Source::Git` creation when `raw` is enabled. `crates/uv-cli/src/lib.rs:4014` documents that `--raw` writes the direct requirement into `project.dependencies` instead of `tool.uv.sources`; `--raw-sources` is an alias.
- astral-sh/uv#13799 explicitly limits its historical fix to the conventional `git` username. The linked export reports predate the reporter's uv 0.12.23 and describe a different transformation: masking with stars instead of dropping a custom account.

## Search scope and exclusions

Before searching, the report was separated into: (1) username loss during `uv add` source serialization; (2) expected reuse of the saved URL for later private-Git updates; and (3) the triggering condition of a non-GitHub SSH host with a non-`git` account. The comments add a request to configure preservation of a shared account. Exact identifiers were `mygituser`, `git+ssh`, `tool.uv.sources` and uv 0.12.23; no concrete authentication error was supplied.

Used authenticated gh to search open and closed issues and all-state pull requests with literal terms ssh/username, drops username, mygituser and username removed; conceptual terms covered Git credential stripping, SSH authentication with pyproject, custom accounts, redaction and username preservation. Fix-oriented searches covered PR username/credentials terms and the historical 0.7.9 reports; inspected candidate bodies, comments, reviews and reference chains through astral-sh/uv#13791 to astral-sh/uv#13799 and from astral-sh/uv#6056 to astral-sh/uv#6074. Ruled out astral-sh/uv#6420 (tool receipts, confirmed fixed in 0.3.1), astral-sh/uv#14147 (stale **** in lockfiles, fixed by 0.7.13) and astral-sh/uv#8529 (broader authentication configuration) as canonical duplicates. PR search commands returned no matches and API searches did not reliably honor PR qualifiers; linked PRs were inspected directly. Further REST searches were rate-limited, so search coverage is incomplete.

The closest excluded candidates were inspected, including comments:

- astral-sh/uv#6420 loses the `git` username from tool installation receipts during `uv tool upgrade`. The reporter confirmed that uninstalling and reinstalling with uv 0.3.1 fixed it. This is a different command and persistence artifact.
- astral-sh/uv#14147 concerns `uv sync --locked` rejecting lockfiles containing `****`. Both reporter and maintainer confirmed that uv 0.7.13 stopped generating that broken state. It does not track custom usernames disappearing from project sources.
- astral-sh/uv#13791 is the broader historical export-obfuscation discussion. Maintainer comments point to astral-sh/uv#13799 for the SSH fix, while an unrelated authenticated-wheel problem was moved elsewhere.
- astral-sh/uv#8529 requests better Git authentication configuration, including environment variables and credential providers. It is broader than retaining the already-supplied shared username, so astral-sh/uv#15034 is the closer discussion.

## Validation and next step

Recommend centralizing the shared-account SSH reproduction in astral-sh/uv#15034. The draft offers the documented `--raw` workaround and makes its different project-file representation explicit. It does not promise a fix or claim the historical export change supported arbitrary usernames.

This handoff is based on issue history, documentation and source inspection. No builds, tests or private-server authentication attempts were run. No checkout files or GitHub objects were modified; only this temporary handoff README was authored.
