# no error when using non-existent `preview` setting

Issue: astral-sh/uv#22950

Classification: question

## Summary

The reporter uses uv 0.13.1 on Windows with `[tool.uv] preview = true` in
`pyproject.toml` and says the setting silently has no effect. They infer that
`preview` was renamed to `preview-features` and ask for an error when using
the old name. After deliberately adding `asdf = true`, `uv sync` warns
about an unknown field and lists both preview settings as accepted fields.
The report does not name the preview feature or show the expected and actual
behavior that led to the original observation.

Runtime reproduction with the installed uv 0.13.5 on Linux confirms that
`uv sync` accepts `preview = true` without an error and that an invalid `asdf`
field produces a warning listing both preview settings. This reproduces the
reported acceptance and diagnostic, but does not establish a bug: both settings
intentionally exist. The legacy boolean remains supported in the reported
release, while the newer name is documented. In the runtime checks, both true
settings enabled identical preview flags; `preview = false` enabled none.
The legacy setting also suppressed the experimental `uv check` warning.

The assertion that the setting silently does nothing is not established for
the reporter's unspecified feature. Some preview features are consulted before
configuration is loaded, but the report does not identify enough behavior to
connect it to that known limitation. Local runtime checks covered Linux and
uv 0.13.5, not Windows or uv 0.13.1. Maintainer EliteTK separately reports
successful tests of `preview = true` on uv 0.13.1, without specifying a
platform or test commands, and recommends using `preview-features` instead.

astral-sh/uv#18437 and astral-sh/uv#15767 explain the intentional coexistence of both settings. astral-sh/uv#19825 tracks a narrower case of ineffective preview configuration, but no duplicate is established.

## Maintainer position and diagnostic policy

In astral-sh/uv#22950, maintainer EliteTK reports that tests on uv 0.13.1
show `preview = true` continues to work, although it is no longer documented
(October 27, 2026, comment ID 6447517469). The recommendation is to encourage
`preview-features` instead. EliteTK does not know whether the legacy option
will ever stop working; this is neither an announced removal plan nor a
promise of indefinite support. This answers the pending clarification about
current behavior and the recommended setting.

This maintainer-reported result agrees with the release-specific source,
historical PR discussion, and local Linux reproduction below. The comment
does not include a platform, commands, or an affected feature, so it does not
verify the reporter's Windows scenario or explain the no-effect observation.
Zanieb's earlier compatibility explanation concerned an unknown individual
preview feature name, as zanieb subsequently clarified; it should not be
used as a decision about the global boolean option.

The reporter questions why a diagnostic would break compatibility when
invalid configuration currently produces only a warning, and points to
astral-sh/uv#22951. That open issue requests fatal `pyproject.toml` parse
errors, or an option to make them fatal, using the same `asdf` diagnostic.
This clarifies a separate question about warning versus error policy; it
does not supply an affected preview feature or establish that the supported
legacy setting should be treated as invalid. The existing Linux reproduction
confirms that the invalid-field example warns and exits 0. The linked issue
also reports that parse failures discard other uv settings, but the recorded
reproduction does not test that broader claim. The supplied discussion has
no substantive maintainer decision on this warning-versus-error objection.

## Draft response

`preview = true` is still supported in uv 0.13.1 and resolves the same way as `preview-features = true`. astral-sh/uv#18437 added the newer setting while retaining the old one for compatibility, so both names in the parser's expected-field list are intentional. In a minimal project tested with uv 0.13.5 on Linux, `uv sync` accepted the legacy setting and `--show-settings` showed the same enabled preview flags for both spellings.

Could you share which preview feature you expected to enable, along with a minimal configuration, command, and expected versus actual output? Some features take effect before configuration files are loaded; missing warnings for that case are tracked in astral-sh/uv#19825. A concrete example will let us determine whether that applies here.

## Classification

The premise that preview no longer exists is contradicted by uv 0.13.1 source and the historical discussion on astral-sh/uv#18437: it remains supported, intentionally appears in the schema, and resolves like preview-features = true. The report does not identify the feature or observable behavior behind 'silently doing nothing', and no separate correctness defect or regression is established. The related early-loading issues cannot be treated as duplicates without that information.

The deliberately invalid `asdf` field demonstrates the parser's accepted
field list; it does not demonstrate that `preview` is invalid or ignored.
This classification does not dismiss the reporter's observation: the missing
information is the particular feature and behavior, not evidence that the
legacy option still exists. If that example establishes incorrect behavior,
classify the resulting defect as a bug or a duplicate of its matching open
issue. No historical fix has been shown to have regressed.

## Reproduction

**Outcome: reproducible**, specifically the absence of an error for
`preview = true` and the unknown-field diagnostic naming both settings.
These are intentional compatibility behavior. The separate assertion that
an unnamed preview feature has no effect remains unverified.

### Environment and isolation

- Reported: Windows, uv 0.13.1; Python version and affected preview feature
  were not supplied. No last known-good version or explicit upgrade sequence
  was provided.
- Maintainer-reported: EliteTK tested `preview = true` successfully on uv
  0.13.1. Platform, Python version, fixture, and commands were not provided;
  this is separate corroboration, not an additional local reproduction.
- Tested locally: Linux x86_64, installed `uv 0.13.5 (x86_64-unknown-linux-gnu)` from
  `/opt/hostedtoolcache/uv/0.13.5/x86_64/uv`, CPython 3.12.3 at
  `/usr/bin/python3`. The checkout is uv 0.13.6 source and was not built.
- All fixtures, virtual environments, cache, temporary, XDG configuration and
  data directories were isolated under `/tmp/uv-22950-vzlk_61f`. Commands used
  a clean environment, offline mode, disabled Python downloads, and the system
  interpreter. No dependencies, workspace members, dependency groups, or
  alternative configuration files were needed.
- Full command output and per-command fixtures are retained in
  `/tmp/uv-22950-vzlk_61f/results.json`; the controlled subprocess environment
  is recorded in `/tmp/uv-22950-vzlk_61f/environment.json`.

### Minimal fixture and commands

Create `pyproject.toml` in an isolated temporary project:

```toml
[project]
name = "preview-repro"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = []

[tool.uv]
preview = true
```

Run with a temporary cache/configuration environment and no inherited preview
flags:

```console
uv sync --offline --python /usr/bin/python3
uv sync --offline --python /usr/bin/python3 --show-settings
```

Both commands exited 0. Sync created `.venv`, resolved the local project,
and completed without a parsing or deprecation warning. The settings output
contained a populated `preview: Preview { flags: [...] }` including `Pylock`,
`FormatCommand`, and `CheckCommand`.

Then add `asdf = true` below `preview = true` and repeat:

```console
uv sync --offline --python /usr/bin/python3
```

This also exited 0, and emitted the reported warning:

```text
warning: Failed to parse `pyproject.toml` during settings discovery:
  TOML parse error at line 9, column 1
    |
  9 | asdf = true
    | ^^^^
```

The following diagnostic identified unknown field `asdf` and listed both
`preview` and `preview-features` among the expected fields. The unknown
`asdf` field is a deliberate invalid-input control; it is not evidence that
`preview` is invalid.

### Controls and interpretation

- With the invalid field removed, replacing `preview = true` with
  `preview-features = true` and repeating the settings command produced an
  identical preview flags block. The complete settings outputs differed only
  in the runtime timestamp used for `exclude-newer`.
- Replacing the setting with `preview = false` produced `flags: []`.
  This directly distinguishes recognized enable-all configuration from an
  ignored setting.
- `uv check --offline --python /usr/bin/python3` emitted the experimental
  `uv check` warning with `preview = false`, but omitted it with
  `preview = true`. Both commands subsequently exited 2 because offline mode
  prevented fetching the uncached ty version manifest. This verifies the
  warning toggle, not successful type checking.
- `uv export --format pylock.toml --offline --python /usr/bin/python3`
  exited 0 and emitted the empty project's pylock output with either boolean
  value, without a preview warning in either case. This command did not
  distinguish the settings and is not evidence of an ignored toggle.

The source and release history support intentional compatibility, rather than
removal of the old field. The documented early-loading limitation remains a
possible explanation only if the reporter supplies an affected feature.
To investigate that claim, request the specific feature, minimal project and
configuration (including any `uv.toml`), exact command, expected versus actual
output, and Python version. The observed Linux result does not establish
Windows-specific behavior or a defect in uv 0.13.1.

### Existing test coverage

`crates/uv/tests/sync/show_settings.rs::preview_precedence` constructs user
`uv.toml` and project `pyproject.toml` configurations and compares settings
snapshots against `--preview` and disabled baselines. It covers project
`preview = true`, equivalence and precedence of `preview-features = true`,
project `preview = false` overriding user enable-all configuration, and CLI
and environment overrides. Its setup and relevant assertions were inspected.
The module requires `test-python` and `test-pypi`; this configuration test is
ignored on Windows. It therefore supports the compatibility contract but
cannot validate the reporter's Windows run or unspecified feature.

No repository tests were added or executed, and no compilation was performed.
Validation used the installed executable directly.

## Related

- astral-sh/uv#18437 — Allow configuring preview features in `uv.toml` and `pyproject.toml` (pull request, merged). Introduced preview-features while retaining preview for compatibility. Maintainer discussion explicitly says the old setting is hidden, not replaced; uv 0.13.1 source resolves both boolean settings identically. This explains the accepted fields, rather than fixing an obsolete setting being silently ignored.
- astral-sh/uv#15767 — Individually specify enabled preview features in configuration (issue, closed). The original request for selecting individual preview features in configuration, implemented by astral-sh/uv#18437. It explains why preview-features was added alongside the existing all-features toggle; it did not request removing preview.
- astral-sh/uv#19825 — Warn when we load a configuration file containing preview settings which were already consulted (issue, open). Tracks missing warnings when preview configuration arrives too late to affect features used during workspace discovery. This is a meaningful adjacent explanation for ineffective configuration, but the reporter identifies no affected feature, so the same underlying problem is not established.

- astral-sh/uv#22951 — `pyproject.toml` parse warnings should be an error instead (issue, open). Linked by the reporter to challenge the compatibility rationale: it requests a nonzero exit for invalid configuration, optionally behind a setting, and uses the same `asdf` warning. It concerns handling genuinely invalid configuration; astral-sh/uv#22950 concerns acceptance of a still-supported preview setting. This is relevant diagnostic-policy context, not an established duplicate or proof that `preview` has no effect.

## Supporting evidence

- The [uv 0.13.1 settings definition](https://github.com/astral-sh/uv/blob/0.13.1/crates/uv-settings/src/settings.rs#L3146)
  explicitly says the legacy setting remains supported and included in the JSON
  schema, while being omitted from generated settings documentation.
- The [uv 0.13.1 preview resolver](https://github.com/astral-sh/uv/blob/0.13.1/crates/uv-settings/src/settings.rs#L3225)
  accepts either setting individually, rejects specifying both together, and
  maps both `preview = true` and `preview-features = true` to
  `Preview::all()`.
- The [uv 0.13.1 CLI settings resolution](https://github.com/astral-sh/uv/blob/0.13.1/crates/uv-cli/src/settings.rs#L235)
  consumes either configuration form and treats both boolean enable-all
  settings equally. This is an active resolution path, not merely a schema
  entry retained without implementation.
- The existing [uv 0.13.1 preview precedence test](https://github.com/astral-sh/uv/blob/0.13.1/crates/uv/tests/sync/show_settings.rs#L4196)
  compares legacy configuration with `--preview`, including the
  `[tool.uv] preview = true` case. The configuration test is ignored on
  Windows, so it is not evidence of a reproduced Windows run.
- astral-sh/uv#18437 merged on June 14, 2026, before the reported release.
  Its discussion explicitly distinguishes hiding the legacy setting from
  replacing it. The author says it remains supported and mutually exclusive
  with the new field; a maintainer expects `preview = true` to continue
  working. The earlier discussion in astral-sh/uv#16452 also explicitly
  chooses to keep the legacy boolean and add the boolean-or-list setting.
- The [uv 0.13.1 preview documentation](https://github.com/astral-sh/uv/blob/0.13.1/docs/concepts/preview.md#L43)
  explains that some features take effect before configuration files are
  loaded. Review comments on astral-sh/uv#18437 demonstrate this for an
  early project-discovery option and point to astral-sh/uv#19825.
  That mechanism is confirmed generally, but is not confirmed for this
  reporter's unspecified feature.

The source in the checkout agrees with these release-specific findings.
The runtime observations in Reproduction independently confirm acceptance,
preview flag resolution, and the invalid-field diagnostic on uv 0.13.5/Linux.
They do not identify the feature behind the reporter's no-effect observation.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues with literal preview, preview-features, unknown field, and settings-discovery diagnostics; conceptual searches covered ignored/silent/no-effect settings, renaming/deprecation, configuration, schema validation, and early loading. Searched open, closed, and merged PRs for preview/configuration/fix/rename; keyword PR searches returned no matches, so supplemented them with the all-state PR listing and issue/PR reference chains. Inspected comments, reviews, and uv 0.13.1 source/tests. No matching regression found. Ruled out astral-sh/uv#18922 (invalid exclude-newer values), astral-sh/uv#20547 (external schema-validator regression), and astral-sh/uv#22878 (deserialization refactor). astral-sh/uv#19824 concerns early configuration loading, which is not established here.

The report was decomposed before searching into (1) an apparently ineffective
legacy preview toggle, (2) acceptance of an allegedly removed option and the
request for an error, and (3) both names appearing in parser diagnostics.
Searches treated observable behavior separately from the proposed
rename/removal cause. Windows and the release number were omitted from
broader searches, then release-specific source was checked.

Additional inspected candidates and distinctions:

- astral-sh/uv#19824 proposes loading non-workspace configuration early enough
  to affect discovery. It is adjacent to the late-loading warning issue, but
  no discovery-related feature is identified in this report.
- astral-sh/uv#18922 concerns a genuinely invalid `exclude-newer` value
  being ignored after a parsing warning. Here, the disputed `preview`
  field is valid, and the reporter deliberately inserted the unrelated
  invalid field only to inspect the accepted names.
- astral-sh/uv#20547 concerns a `fastjsonschema` failure processing a valid
  schema. It does not establish that retaining the legacy preview field is
  incorrect.
- astral-sh/uv#22878 replaces the preview-features deserialization
  implementation and reports equivalent values and diagnostics. It is not a
  fix for the legacy setting becoming ineffective.
- astral-sh/uv#19643 concerns understanding a CLI warning's suggested
  arguments, not configuration discovery or removal of a setting.
- astral-sh/uv#14025 concerns internal process-global preview state, not a
  user-facing global configuration option.

PR keyword searches unexpectedly returned no results despite directly
accessible relevant PRs. The handoff therefore does not claim an exhaustive
PR search; issue timelines, linked discussions, and the REST all-state PR
listing supplied additional candidates.

## Maintainer next step

EliteTK has clarified that the legacy setting works in tests on uv 0.13.1
and that `preview-features` is the recommended setting. No further
clarification is pending on that point; a future removal policy remains
unspecified. The question classification remains supported by this report
and the recorded source and runtime evidence. Changing to the recommended
setting is not an established workaround for the reporter's unexplained
no-effect observation.

The reporter's objection about nonfatal warnings also has no substantive
resolution in the supplied discussion. Consider diagnostic severity for
genuinely invalid configuration with astral-sh/uv#22951, separately from the
global preview option. The existing draft above predates these clarifications
and is retained as context, not as a recommendation to repeat it.

Further investigation of the no-effect claim depends on the reporter
identifying the preview feature and supplying minimal configuration, the exact
command, and expected versus actual behavior. Compare any such example with
astral-sh/uv#19825 before assigning a duplicate or bug classification. The
follow-up does not change the current question classification or the recorded
reproduction findings.
