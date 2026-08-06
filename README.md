# uv audit --ignore-until-fixed=ALL

Issue: astral-sh/uv#20983

Classification: enhancement

## Summary

The reporter wants `uv audit` to report only vulnerabilities for which an upstream fix version is
known. Today they can approximate that result by listing each vulnerability under
`--ignore-until-fixed`, but they propose a special `ALL` value that would suppress every finding
without a known fix.

The current interface is narrower than the report's wording suggests: `--ignore-until-fixed`
accepts vulnerability IDs, not package names. It compares each configured ID with a finding's
primary ID and aliases, suppressing the finding only when its fix-version list is empty. There is no
wildcard behavior, so `ALL` is currently just an unmatched vulnerability ID. The example also says
`ruff audit`; based on the title, requested option, and repository, the intended command is
`uv audit`.

No existing issue or pull request was found that tracks a global conditional-ignore selector or an
equivalent "only findings with known fixes" filter. The closest context is the open audit roadmap,
astral-sh/uv#18506, and the merged implementation of per-ID ignores, astral-sh/uv#18737.

## Draft response

Thanks. `--ignore-until-fixed` currently accepts vulnerability IDs, not package names, and
suppresses each matching vulnerability only while no fix version is known. There is no wildcard
today, so `ALL` would be treated as an unmatched vulnerability ID.

We'll track this as an enhancement to show only findings with known fixes. Before implementation,
we need to decide whether that behavior should be represented by a special `ALL` value or by an
explicit audit filter flag/configuration; the existing option and schema are defined as lists of
vulnerability IDs. Also, the example command should use `uv audit`, not `ruff audit`.

## Classification

This is an enhancement. The implemented behavior is internally consistent with the CLI help,
configuration schema, filtering code, and integration tests: the option accepts explicit
vulnerability IDs or aliases and conditionally suppresses their findings. Treating a value as a
global selector would add a new filtering capability rather than correct behavior that contradicts
the current contract.

The issue is not a duplicate. astral-sh/uv#18506 is the broad roadmap and records the earlier
request for a *specific* vulnerability to be ignored while no fix exists, but neither its roadmap
items nor its relevant comments request a global selector. astral-sh/uv#18737 implemented that
per-ID request and was not opened in response to astral-sh/uv#20983.

## Related

### astral-sh/uv#18506 — Roadmap: `uv audit` (open issue)

This is the canonical roadmap for `uv audit`. Its comments contain the original suggestion for an
"ignore while no fix" option applying to a specific vulnerability, and its checklist points to the
completed ignore implementation. It establishes the feature's design history, but does not track
the new global-selector request.

### astral-sh/uv#18737 — uv audit: `--ignore` and `--ignore-until-fixed` (merged pull request)

This pull request added the exact option that astral-sh/uv#20983 proposes extending. Its summary
defines both ignore options as additive and ID-based, including matching aliases. Its implementation
and tests confirm that `--ignore-until-fixed ID` suppresses only that vulnerability while it has no
known fix and reports it again once a fix exists.

## Supporting evidence

- `crates/uv-cli/src/lib.rs` documents `--ignore-until-fixed` as accepting a vulnerability ID and
  being repeatable.
- `crates/uv/src/commands/project/audit.rs` searches the configured IDs with
  `Vulnerability::matches` and suppresses only a matched finding whose `fix_versions` is empty.
- `crates/uv-audit/src/types.rs` implements matching against the primary vulnerability ID and its
  aliases; it has no special-case selector.
- `uv.schema.json` defines `ignore-until-fixed` as an array of vulnerability-ID strings.
- Integration snapshots in `crates/uv/tests/build/audit.rs` cover suppression with no fix,
  re-reporting when a fix exists, alias/package scoping, configuration, and warnings for unmatched
  IDs.
- astral-sh/uv#20983 has no maintainer comments or additional reproduction. It is currently labeled
  `enhancement`, consistent with the source-backed classification.

## Search coverage and ruled-out candidates

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
included `ignore-until-fixed`, `uv audit`, `ruff audit`, `ALL`, `no fix versions`, and variations of
`ignore while no fix`. Conceptual queries covered wildcard/global or ignore-all behavior,
actionable/fixable vulnerabilities, available fixes, suppressing or filtering audit findings, and
showing only vulnerabilities that can be upgraded. Fix-oriented searches included the original
audit proposal, the audit roadmap, the merged ignore implementation, and automatic-fix work.

The following plausible candidates were inspected but ruled out as same-request matches:

- astral-sh/uv#19265 uses the same option but asks `uv` to remove obsolete ignore entries from user
  configuration after an upgrade. It does not request global filtering.
- astral-sh/uv#19428 requests an automatic `uv audit --fix` workflow that changes the lockfile. It
  shares the goal of acting on fixable vulnerabilities, but not the proposed reporting filter.
- astral-sh/uv#18912 proposes limiting audits to direct dependencies rather than filtering findings
  by fix availability.
- astral-sh/uv#19981 proposes VEX-file support for reasoned vulnerability suppressions, not a
  wildcard for vulnerabilities without fixes.
- astral-sh/uv#9189 is the closed original proposal for the entire `uv audit` command; canonical
  ongoing work moved to astral-sh/uv#18506.
