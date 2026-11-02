# No paging in uv help?

Issue: astral-sh/uv#23053

Classification: question

## Summary

The reporter uses uv 0.13.6 on Debian 13 with `PAGER=less`. Detailed help from
`uv help <cmd>` opens a pager, but bare `uv help` prints directly to the terminal.
They ask whether this is intended or indicates a problem with their system. There is no
error message, report of a previously working version, or explicit request to change the behavior.

astral-sh/uv#4936 intentionally disabled paging for bare `uv help`; astral-sh/uv#5511 contains a maintainer answer to the same question about PAGER=less. The checked-out source still explicitly excludes root help from paging.

## Draft response

This is intentional: bare `uv help` prints the command overview without a pager, while `uv help <command>` pages the detailed help when stdout is a terminal. That distinction was introduced in astral-sh/uv#4936 and confirmed in the discussion on astral-sh/uv#5511. Your observation matches uv's behavior; `$PAGER=less` does not enable paging for the overview. To page it explicitly, use `uv help | less`.

## Classification

The reporter asks whether the difference is intentional. The explicit design rationale in astral-sh/uv#4936, the maintainer clarification in astral-sh/uv#5511, and the current `!is_root` paging condition establish intended behavior. No malfunction or regression is established, and the report does not explicitly request a behavior change. These historical PRs explain the design rather than track an outstanding problem or request to duplicate.

The conclusion is supported by an approved design change and a maintainer's explicit explanation,
not merely by observing how the implementation happens to work. The root-help exception predates
the reported version by more than two years. The initial paging implementation in
astral-sh/uv#4909 was deliberately narrowed by astral-sh/uv#4936 on the same day, July 9, 2024;
this does not establish a regression in uv 0.13.6.

## Related

- astral-sh/uv#4936 — **Do not use pager for root `uv help` and improve after help hint**
  (merged July 9, 2024). Explicitly made bare `uv help` an unpaged, short-form overview. Its rationale describes the root help as a landing page; the current source retains the same exclusion. This directly explains the reported behavior.
  The PR was approved by a maintainer; it has no discussion or inline review comments.
- astral-sh/uv#5511 — **Respect `PAGER` env var when paging in `uv help` command**
  (merged October 1, 2024). Added PAGER support. In its comments, another user reported the same difference with PAGER=less; a maintainer explained that bare `uv help` does not page, and the user confirmed `uv help run` does. Pager selection does not override the root-help exclusion.
  On October 24, 2024, maintainer zanieb wrote: “`uv help` doesn't page because it's really
  short. Does `PAGER='less' uv help run` page?” The reporter confirmed that it did.

## Supporting evidence

- `crates/uv/src/commands/help.rs:49` identifies the root command. The root branch renders
  short help with `render_help()`, while subcommands use `render_long_help()`.
- `crates/uv/src/commands/help.rs:96` sets
  `should_page = !no_pager && !is_root && is_terminal`. The root-command exclusion is
  unconditional with respect to terminal size and the value of `PAGER`.
- `crates/uv/src/commands/help.rs:98` only calls `Pager::try_from_env()` after paging is
  enabled. The environment lookup at line 375 therefore cannot enable paging for bare
  `uv help`.
- The patch in astral-sh/uv#4936 added both the root short-help branch and the
  `!is_root` guards. Its description explicitly says root help is intended as a landing
  page rather than an in-depth display.
- `docs/getting-started/help.md` describes paging for long help, but does not spell out the
  root-command exception. The historical design and current source supply that distinction.
- `crates/uv/tests/it/help.rs:26` snapshots the root overview, and the same module contains
  subcommand and no-pager snapshots. Captured command-output snapshots do not by themselves
  verify interactive pager startup. No runtime reproduction or build was needed for this
  source- and history-backed clarification.

## Search scope and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using literal terms 'uv help', 'pager', 'paging', and 'no paging'; conceptual terms included help with scroll, root, top-level, pagination, and area:cli/page. Also searched the reported version and closed pager/less fixes. PR keyword searches returned no hits, so issue references and help.rs commit history were followed to inspect historical PRs, comments, reviews, and patches. Ruled out astral-sh/uv#16914 and astral-sh/uv#16015 (BusyBox option failures), astral-sh/uv#16878 (missing colors), and astral-sh/uv#16913 (pager prompt text). No matching unresolved defect or regression was found.

The report was decomposed into one observable difference (root help versus subcommand help),
the CLI help/pager subsystem, the interactive-terminal context, and the exact identifiers
`uv help`, `PAGER=less`, uv 0.13.6, and Debian 13. Searches without version and platform
constraints checked the underlying behavior separately from possible pager-selection or
compatibility failures.

Additional inspected leads:

- astral-sh/uv#4931 requested support for selecting a pager through `PAGER` and was closed
  by astral-sh/uv#5511. It does not request paging of the root overview.
- astral-sh/uv#4941 requested `--no-pager`; astral-sh/uv#5221 concerned displaying help-specific
  options. Neither tracks automatic paging of bare `uv help`.
- astral-sh/uv#16914 concerns an invoked BusyBox pager rejecting `-R`, and astral-sh/uv#16015
  concerns rejection of `-P`. The latter's fix, astral-sh/uv#16908, changes the pager header
  and color handling. Here, subcommand paging succeeds and root help deliberately never invokes
  the pager.
- astral-sh/uv#16878 concerns colors being removed from paged help; astral-sh/uv#16913 requests
  persistent command information in the pager prompt. Both assume the pager is already running.

The PR keyword searches were inconclusive despite known historical PRs being directly accessible.
The closest results were verified through issue links, the help implementation's commit history,
and direct PR reads, including the relevant comments and patches.
