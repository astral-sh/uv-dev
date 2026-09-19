# Provide a way to add/remove additional requirements after installation of tools

Issue: astral-sh/uv#21833

Classification: duplicate

## Summary

The issue requests a low-friction way to add, update, or remove additional requirements associated
with an installed tool. The motivating case is a plugin-based application whose plugins must share
the tool environment and are discovered incrementally. Although `uv tool list --show-with` exposes
the current additional requirements, the reporter wants uv to retain the rest of the installation
configuration and update that set without requiring the user to reconstruct the full original
`uv tool install` invocation.

astral-sh/uv#19980 is the canonical open tracker for this request. It was opened by a maintainer to
track a flexible, declarative way to install tools with modifications without remembering a long
list of `--with` options. astral-sh/uv#18996 and astral-sh/uv#15792 cover the narrower add and remove
workflows. The earlier astral-sh/uv#14746 proposed add, upgrade, and uninstall operations against an
installed tool environment; it was closed after astral-sh/uv#19980 was opened to track the preferred
sound, declarative design.

No matching implementation pull request was found.

## Draft response

Thanks for the detailed use case. This is already tracked in astral-sh/uv#19980, which covers
changing a tool's additional requirements without having to reconstruct the complete prior
`--with` invocation. The add-only workflow is also described in astral-sh/uv#18996, and the removal
behavior is discussed in astral-sh/uv#15792.

The current design direction is to keep tool environments declarative and fully resolved rather
than mutate them with an imperative injection operation, since in-place changes can leave the tool
and plugin requirements inconsistent. The exact interface still needs design and a decision;
astral-sh/uv#19980 is the place to continue that discussion. Closing this as a duplicate.

## Classification

Duplicate. astral-sh/uv#19980 tracks the same underlying enhancement: provide a sound, declarative
way to modify a tool installation's extra requirements without reconstructing the entire prior
`uv tool install ... --with ...` command. The explicit add, update, and remove command sketches and
the OCRmyPDF plugin example add useful motivation, but do not require a separate tracker.

This is an enhancement request rather than a correctness report in isolation, but the duplicate
classification takes precedence because the canonical issue remains open.

## Related

- astral-sh/uv#19980 — Canonical maintainer-opened tracker for changing a tool's additional
  requirements without remembering or retyping the complete `--with` list. It explicitly favors a
  declarative, fully resolved design over imperative injection and remains open with `needs-design`
  and `needs-decision` labels.
- astral-sh/uv#18996 — Open enhancement covering the add/update half of the request: retain the
  installed tool's existing additional packages while appending another package.
- astral-sh/uv#15792 — Open question covering the removal half. Maintainer evidence confirms that
  reinstalling with a new `--with` set removes requirements omitted from that new set, and the
  follow-up identifies the burden of repeating every requirement that should remain.
- astral-sh/uv#14746 — Closed earlier proposal for post-install add, upgrade, and uninstall
  operations. Maintainers explained that direct mutation can produce an inconsistent resolution,
  accepted that an immutable declarative workflow could address the use case, and closed the issue
  immediately after opening astral-sh/uv#19980.

## Search evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal searches
used `uv tool install`, `--with`, additional dependencies, add/remove/update, reinstalling options,
and tool environments. Conceptual searches used plugins, injection and `pipx inject`, declarative
tool configuration, receipts, upgrade, sync, and preservation of prior requirements. Fix-oriented
searches looked for implementations involving `--show-with`, tool receipts, and tool-environment
modification.

The reference chain from astral-sh/uv#14746 through astral-sh/uv#7312 led to astral-sh/uv#19980.
astral-sh/uv#14143 was inspected because it asks `--reinstall` to remember an installation, but it is
not the canonical match: it focuses on recreating the original setup after an interpreter change,
not editing the saved additional-requirement set. astral-sh/uv#7312 established initial installation
with `--with` as uv's alternative to `pipx inject`, but it does not address incremental changes.
