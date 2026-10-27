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

Both settings intentionally exist. In the reported release, the legacy
boolean setting remains supported and resolves identically to the boolean
form of `preview-features`. The newer name is documented while the old name
remains in the schema for compatibility. There is a separate, known problem
with preview settings loaded after the operations they affect, but this
report does not identify enough behavior to establish that relationship.

astral-sh/uv#18437 and astral-sh/uv#15767 explain the intentional coexistence of both settings. astral-sh/uv#19825 tracks a narrower case of ineffective preview configuration, but no duplicate is established.

## Draft response

`preview = true` is still supported in uv 0.13.1 and resolves the same way as `preview-features = true`. astral-sh/uv#18437 added the newer setting while retaining the old one for compatibility, so both names in the parser's expected-field list are intentional.

Could you share which preview feature you expected to enable, along with a minimal configuration, command, and expected versus actual output? Some features take effect before configuration files are loaded; missing warnings for that case are tracked in astral-sh/uv#19825. A concrete example will let us determine whether that applies here.

## Classification

The premise that preview no longer exists is contradicted by uv 0.13.1 source and maintainer discussion: it remains supported, intentionally appears in the schema, and resolves like preview-features = true. The report does not identify the feature or observable behavior behind 'silently doing nothing', and no separate correctness defect or regression is established. The related early-loading issues cannot be treated as duplicates without that information.

The deliberately invalid `asdf` field demonstrates the parser's accepted
field list; it does not demonstrate that `preview` is invalid or ignored.
This classification does not dismiss the reporter's observation: the missing
information is the particular feature and behavior, not evidence that the
legacy option still exists. If that example establishes incorrect behavior,
classify the resulting defect as a bug or a duplicate of its matching open
issue. No historical fix has been shown to have regressed.

## Related

- astral-sh/uv#18437 — Allow configuring preview features in `uv.toml` and `pyproject.toml` (pull request, merged). Introduced preview-features while retaining preview for compatibility. Maintainer discussion explicitly says the old setting is hidden, not replaced; uv 0.13.1 source resolves both boolean settings identically. This explains the accepted fields, rather than fixing an obsolete setting being silently ignored.
- astral-sh/uv#15767 — Individually specify enabled preview features in configuration (issue, closed). The original request for selecting individual preview features in configuration, implemented by astral-sh/uv#18437. It explains why preview-features was added alongside the existing all-features toggle; it did not request removing preview.
- astral-sh/uv#19825 — Warn when we load a configuration file containing preview settings which were already consulted (issue, open). Tracks missing warnings when preview configuration arrives too late to affect features used during workspace discovery. This is a meaningful adjacent explanation for ineffective configuration, but the reporter identifies no affected feature, so the same underlying problem is not established.

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
No build, integration test, or runtime reproduction was performed; the
conclusion rests on source, existing test assertions, and maintainer
discussion.

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

Use the draft response to clarify that the legacy option is accepted
intentionally and request the specific feature, minimal configuration,
command, and expected versus actual behavior. Compare that example with
astral-sh/uv#19825 before assigning a duplicate or bug classification.
This is a proposed response only; nothing has been posted to GitHub.
