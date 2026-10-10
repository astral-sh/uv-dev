# Update SchemaStore `uv.json` to include `audit.malware-check` and `audit.malware-check-url`

Issue: astral-sh/uv#22458

Classification: bug

## Summary

The reported SchemaStore omission was reproduced on October 10, 2026. The downloaded
published `uv.json` and SchemaStore source omitted `malware-check` and `malware-check-url` from
`AuditOptions.properties`. A targeted JSON Schema validation accepts invalid Boolean,
URL-type, and URL-format values that uv's generated schema rejects. Both schemas
accept the reporter's valid configuration, and installed uv 0.12.13 accepts it during
an offline lock operation.

These settings shipped in uv 0.11.31 on July 21, 2026, through astral-sh/uv#20587.
The issue concerns publishing definitions for existing settings, including completion
metadata and type constraints. It does not establish a runtime uv defect or a
particular editor diagnostic.

A maintainer linked SchemaStore/schemastore#6485, an open PR that refreshes the external
schema and adds both missing properties. Its diff confirms coverage of this report;
merge and publication are not yet established. The next step is to follow that PR and
verify the published schema after the update lands.

## Classification

Bug: incomplete external schema coverage for supported configuration. The published
`AuditOptions` allows additional properties, so its omission leaves these values
unvalidated instead of necessarily rejecting valid configuration. Validation behavior
was reproduced directly; editor completion was not exercised because no editor,
extension, or version was supplied.

There is no evidence that these properties were previously published and then removed.
Earlier SchemaStore refreshes predate their introduction, so this is not an established
regression of those fixes. No additional runtime configuration feature is needed.
SchemaStore/schemastore#6485 supplies a proposed fix following this report; its existence
does not make the issue a duplicate or establish that the published schema is fixed.

## Reproduction

Outcome: **reproducible**, verified October 10, 2026.

### Environment and scope

- Linux 6.17.0-1022-azure, x86_64.
- Installed executable on PATH: `/opt/hostedtoolcache/uv/0.12.13/x86_64/uv`,
  reporting `uv 0.12.13 (x86_64-unknown-linux-gnu)`.
- CPython 3.12.3 at `/usr/bin/python3`.
- `jsonschema` 4.10.3, `Draft7Validator`, with `FormatChecker` and the optional
  `rfc3987` 1.3.8 URI checker installed only in the temporary reproduction directory.
- Local schema from checkout commit `44b2e5877ef752e360acc02c7d198ba71ddf948d`.
- Report supplies the configuration below, but no uv/Python versions, platform,
  command failure, editor version, or specific editor diagnostic.
- All reproduction inputs, downloaded schemas, outputs, and caches are under
  `/tmp/uv-22458-dcm0d2ll`. No checkout files or GitHub state were changed.

### Runtime acceptance of the reported configuration

Created `project/pyproject.toml` under the temporary directory:

```toml
[project]
name = "schema-repro"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = []

[tool.uv.audit]
malware-check = true
malware-check-url = "https://example.com"
```

From that project directory, ran the installed executable:

```sh
uv lock --offline --no-python-downloads --python /usr/bin/python3
```

The subprocess used a clean environment with PATH retained and `UV_CACHE_DIR`,
`UV_PYTHON_INSTALL_DIR`, `XDG_CONFIG_HOME`, `XDG_CONFIG_DIRS`, and `TMPDIR` pointing
inside the temporary directory. It exited 0:

```text
Using CPython 3.12.3 interpreter at: /usr/bin/python3
Resolved 1 package in 1ms
```

This confirms configuration acceptance. It does not exercise a malware-service request;
the command was offline and the project had no dependencies.

### Published-schema validation

Downloaded these public resources directly, without authentication:

- https://json.schemastore.org/uv.json
- https://raw.githubusercontent.com/SchemaStore/schemastore/master/src/schemas/json/uv.json
- https://json.schemastore.org/pyproject.json

The first two files were identical, SHA-256
`7bbc3c50810a097efdcd8be3981b2c73c91b06eaabbdf3b9729462b636d4d99a`.
Their `AuditOptions.properties` contained only `ignore` and `ignore-until-fixed`;
`additionalProperties` was omitted. The published pyproject schema references
`uv.json` at `properties.tool.properties.uv`.

Validated the parsed TOML's `tool.uv` object against the complete published uv schema
and against a temporary copy of the checkout's generated `uv.schema.json`.
The validation script and JSON results are retained as `validate_schema.py` and
`validation-results.json` in the temporary directory. The essential comparison is:

```python
import json
from pathlib import Path
from urllib.request import urlopen
from jsonschema import Draft7Validator, FormatChecker

with urlopen("https://json.schemastore.org/uv.json", timeout=30) as response:
    published_schema = json.load(response)
local_schema = json.loads(
    Path("/home/runner/work/uv/uv/uv.schema.json").read_text()
)
published = Draft7Validator(published_schema, format_checker=FormatChecker())
local = Draft7Validator(local_schema, format_checker=FormatChecker())

def compare(audit):
    instance = {"audit": audit}
    print(published.is_valid(instance), local.is_valid(instance))

compare({"malware-check": True, "malware-check-url": "https://example.com"})
compare({"malware-check": "yes", "malware-check-url": "https://example.com"})
compare({"malware-check": True, "malware-check-url": 123})
compare({"malware-check": True, "malware-check-url": "not a URL"})
```

Run with `jsonschema` and its URI checker available. The retained script can be rerun with:

```sh
PYTHONPATH=/tmp/uv-22458-dcm0d2ll/validation-packages \
  python3 /tmp/uv-22458-dcm0d2ll/validate_schema.py
```

Observed results:

| Configuration | Published SchemaStore schema | Local uv schema |
| --- | --- | --- |
| Reported Boolean and HTTPS URL | Accepts | Accepts |
| `malware-check = "yes"` | Accepts | Rejects |
| `malware-check-url = 123` | Accepts | Rejects |
| `malware-check-url = "not a URL"` | Accepts | Rejects with URI checking enabled |

The initial system validator lacked an optional URI checker and accepted the malformed
URL even with the local schema. Installing `rfc3987` into the temporary target enabled
that check and produced the final result above. Type rejection did not require this
optional dependency. This distinction matters for editor-specific format validation.

### Existing test coverage

Read the setup and snapshot of
`crates/uv/tests/sync/sync.rs::sync_malware_detected`. It sets both properties in
`[tool.uv.audit]`, points the URL to a mock OSV service, removes the corresponding
environment overrides, and asserts that a mocked malware advisory aborts sync with
exit code 2. This covers runtime use of both file settings, not external schema publication.
The `sync` module is gated by `test-python` and `test-pypi` in
`crates/uv/tests/sync/main.rs`.

Searches of `crates/uv/tests/` and `crates/uv-client/tests/it/` found no integration
coverage for these fields in the published SchemaStore schema. The repository's
`scripts/validate-pyproject.sh` validates using the checked-in schema, so it does not
check whether SchemaStore's separate copy includes the fields. No Rust tests were
added or run; the targeted schema validation and installed-uv command are the
behavioral reproductions.

## Related

- SchemaStore/schemastore#6485 — Update uv's JSON schema (open). Linked by maintainer charliermarsh on astral-sh/uv#22458. The PR refreshes `src/schemas/json/uv.json` from uv commit `44b2e5877ef752e360acc02c7d198ba71ddf948d`, the same commit used for the local-schema reproduction. Its diff adds both malware properties to `AuditOptions`, directly addressing the reported omission. Merge and publication remain pending verification.
- astral-sh/uv#20587 — Add audit malware-check configuration settings (merged). Added both requested settings and their generated schema definitions, merged July 21, 2026, and included in uv 0.11.31. Confirms these are existing supported settings; it did not update SchemaStore's separate copy.
- astral-sh/uv#20497 — Add a `malware-check` boolean setting as the configuration equivalent of the `UV_MALWARE_CHECK` environment variable (closed). Original request for file-based malware-check configuration, closed by astral-sh/uv#20587. That runtime capability is implemented; the new report concerns publishing its schema definitions.
- astral-sh/uv#17173 — Update UV's JSON Schema on JSON Schema Store (0.9.18) (closed). Earlier SchemaStore lag caused editor validation errors for relative-time settings. A maintainer linked SchemaStore/schemastore#5232, merged December 18, 2025. This establishes the refresh workflow, but predates both malware settings and does not track their omission.
- astral-sh/uv#16545 — Update UV's JSON Schema on JSON Schema Store (closed). Earlier stale-schema report affecting Even Better TOML, resolved through SchemaStore/schemastore#5091 in November 2025. Its linked discussion concerned torch-backend; it is historical publication context rather than a duplicate of the current audit-property omission.

## Supporting evidence

- The October 10, 2026 maintainer comment on astral-sh/uv#22458 links directly to
  SchemaStore/schemastore#6485. Inspection of that PR found state `OPEN`, no merge
  timestamp, and head commit `d954daa9978b1c4112377ed11dda1e2700a51799`.
  The schema diff adds `malware-check` with type `["boolean", "null"]` and
  `malware-check-url` with a `DisplaySafeUrl` reference or null. This confirms a
  proposed schema fix, not deployment of the updated published schema.
- `crates/uv-settings/src/settings.rs:3018` defines `AuditOptions` with
  `malware_check: Option<bool>` and `malware_check_url: Option<DisplaySafeUrl>`;
  serde maps names to kebab case.
- `uv.schema.json:674` defines `AuditOptions`; `malware-check` accepts Boolean or
  null, and `malware-check-url` accepts `DisplaySafeUrl` or null. The URL definition
  specifies a string with `format: "uri"`.
- `docs/concepts/projects/sync.md:236` documents enabling checks through
  `audit.malware-check`; line 239 documents choosing the service through
  `audit.malware-check-url`.
- `changelogs/0.11.x.md:1357` lists both settings in uv 0.11.31, released July 21,
  2026, through astral-sh/uv#20587.
- `scripts/update_schemastore.py:53` reads the generated schema, sets its SchemaStore
  identifier, and writes `src/schemas/json/uv.json` in a SchemaStore checkout. The
  script includes commits and pushes and was not executed.

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

Follow SchemaStore/schemastore#6485 through review and merge. After publication,
fetch https://json.schemastore.org/uv.json again and rerun the retained schema comparison.
Verify that `AuditOptions` exposes both definitions, the valid example still passes,
and invalid Boolean, URL-type, and URL-format values fail validation with URI checking
enabled. The linked PR already proposes the refresh; another refresh PR is unnecessary
unless that proposal changes or fails to address the omission. Editor-specific completion
remains untested. No fix or publication was performed as part of this reproduction.

## Draft response

Reproduced with the current published SchemaStore schema: it omits both properties
and accepts invalid values that uv's generated schema rejects. Your valid configuration
is accepted by uv 0.12.13. Both settings have been supported since uv 0.11.31 through
astral-sh/uv#20587; SchemaStore's separate copy needs a refresh. The omission removes
completion metadata and validation constraints, although it does not itself make the
valid example fail JSON Schema validation.
