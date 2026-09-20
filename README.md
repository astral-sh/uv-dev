# Add a series of  `uv workspace xxx` commands for the workspace feature

Issue: astral-sh/uv#21849

Classification: enhancement

## Summary

The issue proposes adding a `uv workspace` alias hierarchy for existing top-level project commands:
`init`, `add`, `remove`, `sync`, `lock`, `run`, `tree`, `build`, and `publish`. The stated goal
is to make workspace functionality easier to discover and to provide a clearer workspace-management
surface. It does not report a failure or incorrect result from an existing command.

No existing issue or pull request was found that tracks this complete alias hierarchy. The closest
precedent is astral-sh/uv#5702: its discussion explicitly categorized `init`, `add`, `remove`,
`run`, `sync`, `lock`, and `tree` as workspace operations. That issue was resolved by merged
astral-sh/uv#5830, which reordered the existing top-level commands rather than placing aliases under
`uv workspace`. Open astral-sh/uv#18215 is also adjacent, but it requests documentation for the
existing workspace inspection commands rather than new management aliases.

## Draft response

Thanks for the proposal. The existing project commands already operate in workspace contexts, while
`uv workspace` currently exposes workspace inspection commands. The closest prior CLI discussion,
astral-sh/uv#5702, grouped many of these commands conceptually as workspace operations, but
astral-sh/uv#5830 addressed discoverability by ordering the top-level commands rather than nesting or
aliasing them.

We'll treat this as an enhancement proposal. Adding aliases for the full set would need design
consensus, particularly because commands such as `init`, `add`, `remove`, `run`, `build`, and
`publish` can target a project or member rather than the workspace as a whole. Concrete examples
where the current top-level commands are unclear, along with the expected whole-workspace versus
member-specific behavior for each proposed command, would help define the design.

## Classification

This is an enhancement because it requests a new CLI command hierarchy and aliases to improve
discoverability. The report does not establish incorrect existing behavior. Although astral-sh/uv#5702
contains a closely related command-grouping discussion, it tracked help-menu ordering and was closed
after the commands were reordered; it did not request or implement the proposed `uv workspace`
aliases. No open issue or pull request was found that is close enough to centralize this request as a
duplicate.

## Related

- astral-sh/uv#5702 (closed issue), “Improve ordering of top-level commands” — The discussion
  explicitly grouped seven of the proposed commands as workspace operations. The accepted scope was
  top-level help ordering, not a `uv workspace` alias hierarchy; it also did not cover `build` or
  `publish`.
- astral-sh/uv#5830 (merged pull request), “Improve display order of top-level commands” — This
  implemented astral-sh/uv#5702 by reordering the existing top-level commands. It is relevant
  precedent for the discoverability motivation, but it did not nest or alias commands.
- astral-sh/uv#18215 (open issue), “Missing documentation for uv workspace commands” — This asks for
  consolidated documentation for the existing workspace inspection namespace. It is adjacent to the
  discoverability concern but does not request aliases for project-management commands.

## Search evidence

Literal searches covered each proposed form — `uv workspace init`, `add`, `remove`, `sync`, `lock`,
`run`, `tree`, `build`, and `publish` — across open and closed issues and open, closed, and merged pull
requests. Conceptual searches covered workspace commands and subcommands, aliases, namespaces, CLI
grouping, project namespaces and subcommands, workspace management, and monorepo tooling.

The strongest candidates and their comments or linked changes were inspected. Astral-sh/uv#9626 was
ruled out because it requests affected-package builds, dependency-change tracking, and automated
version bumps rather than CLI aliases. Astral-sh/uv#12541 and astral-sh/uv#13636 concern individual
workspace inspection capabilities (`metadata` and `dir`). Astral-sh/uv#18368 attempted documentation
changes related to astral-sh/uv#18215, but it was closed without merge and does not implement the
requested aliases.
