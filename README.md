# Update SchemaStore `uv.json` to include `audit.malware-check` and `audit.malware-check-url`

Issue: astral-sh/uv#22458

Classification: bug

## Summary

The reporter requests SchemaStore definitions for two existing uv settings under
`[tool.uv.audit]`: `malware-check` (Boolean) and `malware-check-url` (URL string).
The expected behavior is editor completion and type validation for this supported configuration:

```toml
[tool.uv.audit]
malware-check = true
malware-check-url = "https://example.com"
```

The omission is confirmed in both SchemaStore's current source and its published schema.
Both definitions already exist in uv's generated schema and Rust configuration types, and the
changelog records their release in uv 0.11.31 on July 21, 2026.

astral-sh/uv#20587 implemented the settings requested in astral-sh/uv#20497. astral-sh/uv#17173 and astral-sh/uv#16545 document earlier SchemaStore refreshes. The current omission is confirmed; none of these items is a duplicate target.

## Draft response

Your example uses settings supported since uv 0.11.31, added in astral-sh/uv#20587. Both are present in uv.schema.json, but SchemaStore's current uv.json omits them from AuditOptions. The next step is to refresh src/schemas/json/uv.json in SchemaStore/schemastore from uv's generated schema, including the Boolean and URI definitions. The repository's scripts/update_schemastore.py provides the existing update workflow.

## Classification

Both settings shipped in uv 0.11.31 and exist in the source and generated schema, but SchemaStore's current source and published AuditOptions omit them. This is incomplete schema coverage for supported configuration, affecting completion and type validation. The original feature request is already implemented, and earlier schema refreshes predate these fields; no existing item was found tracking this specific omission or a prior fix for it.

The report requests correctness for existing settings rather than a new runtime capability.
The source comparison establishes missing schema definitions without requiring an editor
reproduction. No particular editor diagnostic was supplied or reproduced: missing definitions
must not be described as proof that every editor rejects the valid example.
The published `AuditOptions` does not explicitly forbid additional properties, so absent type
constraints and completion metadata are the confirmed defects.

The 2025 refreshes fixed other settings before the malware options were introduced in July 2026.
There is no evidence that these specific properties were previously published and then removed,
so this is not an established regression of those historical fixes. The implementation PR
also predates the new issue and addressed runtime configuration, not this SchemaStore omission.

## Related

- astral-sh/uv#20587 — Add audit malware-check configuration settings (merged). Added both requested settings and their generated schema definitions, merged July 21, 2026, and included in uv 0.11.31. Confirms these are existing supported settings; it did not update SchemaStore's separate copy.
- astral-sh/uv#20497 — Add a `malware-check` boolean setting as the configuration equivalent of the `UV_MALWARE_CHECK` environment variable (closed). Original request for file-based malware-check configuration, closed by astral-sh/uv#20587. That runtime capability is implemented; the new report concerns publishing its schema definitions.
- astral-sh/uv#17173 — Update UV's JSON Schema on JSON Schema Store (0.9.18) (closed). Earlier SchemaStore lag caused editor validation errors for relative-time settings. A maintainer linked SchemaStore/schemastore#5232, merged December 18, 2025. This establishes the refresh workflow, but predates both malware settings and does not track their omission.
- astral-sh/uv#16545 — Update UV's JSON Schema on JSON Schema Store (closed). Earlier stale-schema report affecting Even Better TOML, resolved through SchemaStore/schemastore#5091 in November 2025. Its linked discussion concerned torch-backend; it is historical publication context rather than a duplicate of the current audit-property omission.

## Supporting evidence

- `crates/uv-settings/src/settings.rs:3018` defines `AuditOptions`, with
  `malware_check: Option<bool>` and `malware_check_url: Option<DisplaySafeUrl>`.
  Serde maps their names to kebab case.
- `uv.schema.json:674` defines `AuditOptions`; its two malware properties begin at
  lines 691 and 695. The Boolean accepts `boolean` or `null`; the URL accepts
  `DisplaySafeUrl` or `null`. `DisplaySafeUrl` has JSON type `string` and format
  `uri` at lines 939–940.
- `docs/concepts/projects/sync.md:236` documents enabling checks through
  `audit.malware-check`; line 239 documents choosing the service through
  `audit.malware-check-url`.
- `changelogs/0.11.x.md:1357` lists both settings under uv 0.11.31, released July 21,
  2026. The schema diff in astral-sh/uv#20587 adds both properties; its file list
  contains uv source, tests, documentation, and the local schema.
- On October 10, 2026, https://json.schemastore.org/uv.json and
  https://raw.githubusercontent.com/SchemaStore/schemastore/master/src/schemas/json/uv.json
  both defined only `ignore` and `ignore-until-fixed` within `AuditOptions.properties`.
  Neither defined either malware setting there.
- https://json.schemastore.org/pyproject.json references `uv.json` from
  `properties.tool.properties.uv`, confirming that this published copy applies to
  `[tool.uv.audit]` in `pyproject.toml`.
- `scripts/update_schemastore.py:53` reads uv's generated schema, sets its SchemaStore
  identifier, and writes `src/schemas/json/uv.json` in a SchemaStore checkout. The
  script also commits and pushes changes, so it was inspected but not executed.
- Maintainer comments on astral-sh/uv#16545 and astral-sh/uv#17173 link to
  SchemaStore/schemastore#5091 and SchemaStore/schemastore#5232 respectively.
  Both linked PRs were inspected and are merged; each updates the external
  `src/schemas/json/uv.json` from a uv commit.

## Search coverage and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered each malware setting, dotted audit names, AuditOptions, uv.json, and SchemaStore; conceptual searches covered schema validation, autocomplete/completion, editor support, outdated schemas, and audit configuration; fix-oriented searches covered schema updates and update_schemastore. Inspected candidate bodies and comments, the implementation diff, and linked SchemaStore/schemastore#5091 and SchemaStore/schemastore#5232. Also searched SchemaStore PRs for uv. PR searches omitted the directly retrievable astral-sh/uv#20587, so negative PR results are not conclusive. Ruled out astral-sh/uv#11180 (incorrect conflicts representation), astral-sh/uv#3549 (repeated editor completions), astral-sh/uv#21484 (lockfile integrity), and astral-sh/uv#18506 (broader audit roadmap) as duplicate targets.

The report was decomposed before searching into the two missing properties, the
`AuditOptions`/SchemaStore publication subsystem, and completion/type-validation behavior
triggered by valid `[tool.uv.audit]` configuration. Searches for these observable gaps were
kept separate from searches for publication and refresh mechanisms.

The discussion linked from astral-sh/uv#16545 was followed to astral-sh/uv#12994:
its schema-related comment concerned `torch-backend`, while the main issue requests
automatic backend selection in `uv add`. It does not track the audit fields.
The relevant comments on astral-sh/uv#18506 propose file-based malware configuration;
astral-sh/uv#20497 and astral-sh/uv#20587 supply the focused request and implementation.

## Recommended next step

Refresh SchemaStore's `src/schemas/json/uv.json` from an appropriate released uv schema
containing both fields, and validate that the resulting schema exposes the Boolean and URI
definitions through `AuditOptions`. The existing update script and the historical upstream
PRs document that workflow. No additional runtime configuration feature is needed to address
this report.
