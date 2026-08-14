# Allow sharing exclude-newer-package across workspaces: file reference, env var, or repo-level config

Issue: astral-sh/uv#20995

Classification: duplicate

## Summary

The report asks for a repository-maintained way to define `exclude-newer-package` exemptions once
and reuse them across many independent uv workspace roots. Its concrete monorepo applies a
seven-day cooldown but must exempt about 39 first-party packages whose internal index does not
provide PEP 700 upload times. Today, the same exemption table is copied into 37 manifests and kept
consistent by custom CI.

The reporter would accept any of three mechanisms: a referenced configuration file, an
environment-variable representation suitable for CI and `uvx`, or opt-in merging of a
repository-level configuration above the individual workspace roots.

The underlying repository-local configuration-sharing request is already tracked by
astral-sh/uv#11070. That issue describes independent monorepo projects whose child `[tool.uv]`
configuration prevents them from inheriting shared settings from the root and proposes explicit
opt-in inheritance. astral-sh/uv#5596 tracks the closely related general-purpose `extend` design.
The new issue contributes a detailed `exclude-newer-package` use case and a package-specific
environment-variable alternative, but those are additional design constraints on the existing
sharing request.

## Draft response

Thanks for the concrete monorepo case. The underlying request to share repository-local uv
configuration across independent projects is already tracked in astral-sh/uv#11070, including an
opt-in root-config inheritance design; a broader extend/file-reference mechanism is discussed in
astral-sh/uv#5596. Environment-variable support for per-package exemptions was also raised during
astral-sh/uv#16854, but was not implemented there.

Let's centralize the configuration-sharing discussion in astral-sh/uv#11070. The scale here and the
`uvx`/CI environment-variable use case are useful constraints for that design.
astral-sh/uv#20788 would only shorten prefix-based lists, while astral-sh/uv#19864 concerns bypass
semantics rather than sharing.

## Classification

This is a duplicate because astral-sh/uv#11070 already tracks the same underlying problem: uv
configuration stored in a monorepo root cannot be inherited by independent child projects that
have their own `[tool.uv]` configuration. It also proposes the same class of opt-in inheritance
solution and records the important limitation that user- and system-level merging does not provide
repository-controlled team configuration.

astral-sh/uv#5596 independently tracks the broader external/shared-file solution through a
Ruff-style `extend` option. The proposed `UV_EXCLUDE_NEWER_PACKAGE` representation is a more
specific alternative interface, not an established bug in current behavior. No evidence indicates
a regression of a previously supported merge behavior.

## Related

- astral-sh/uv#11070 (open issue) — The closest canonical request. It asks for opt-in inheritance
  of root-level `[tool.uv]` settings by independent projects in a monorepo. A maintainer notes that
  arbitrary inheritance adds complexity and that existing user/system merging does not address
  team sharing.
- astral-sh/uv#5596 (open issue) — Requests a general `extend` directive for shared uv
  configuration, directly covering the report's external-file solution. Maintainer comments set
  no near-term implementation expectation.
- astral-sh/uv#16854 (merged pull request) — Added the `<name> = false` per-package sentinel on
  which this report relies. Its discussion also contains an earlier request for an environment
  representation, but the pull request did not implement one.
- astral-sh/uv#20788 (open issue) — Requests glob keys for `exclude-newer-package`. This could
  shorten prefix-based exemption tables, but it would not centralize policy across workspace roots;
  maintainer comments also express reservations about package-prefix ownership.
- astral-sh/uv#19864 (open issue) — Proposes structured cooldown bypasses for direct pinned or
  constraining requirements. It changes bypass semantics, not the storage or reuse of configuration.

## Search evidence

Searches covered the exact `exclude-newer-package`, `UV_EXCLUDE_NEWER_PACKAGE`, and
`UV_EXCLUDE_NEWER` identifiers; environment-variable and CLI representations; and conceptual
terms including shared configuration, monorepo and workspace roots, repository-level config,
parent `uv.toml`, inheritance, include, and extend. Open and closed issues and open, closed, and
merged pull requests were considered, and the strongest candidates' comments and cross-references
were followed.

The historical discovery bug astral-sh/uv#5929 and its merged fix astral-sh/uv#5931 were inspected
but ruled out as the canonical request. That fix searches beyond a workspace root when no nearer
project configuration wins; it does not merge arbitrary repository-level configuration with each
workspace's own `[tool.uv]`. Closed astral-sh/uv#10353 was also ruled out because its reporter found
that parent discovery already worked in a case that did not establish merging with child
configuration.
