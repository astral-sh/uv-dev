# `uv add` fails with "Expected `--hash`"

Issue: astral-sh/uv#20931
Classification: duplicate

## Summary

The report is a duplicate of open canonical issue astral-sh/uv#3636, corroborated by the exact prior duplicate astral-sh/uv#13375. Package-scoped settings were added separately by astral-sh/uv#14573, but requirements-file syntax remains unsupported.

## Classification

astral-sh/uv#3636 already tracks the same underlying missing capability: parsing per-requirement `--config-settings` from requirements files. astral-sh/uv#13375 demonstrates the identical error and was explicitly redirected there by a maintainer. The `uv add -r` entry point and clearer-error concern provide another reproduction and UX detail, but do not establish a regression because requirements-file support was never implemented.

## Related

- https://github.com/astral-sh/uv/issues/3636 (open issue): Add support for config_settings in requirements.txt / requirements.in
  Canonical match: astral-sh/uv#3636 tracks support for package-specific `--config-settings` in requirements files, including the same standalone-directive parse error. Maintainers confirmed the capability needs design.
- https://github.com/astral-sh/uv/issues/13375 (closed issue): Support flags in requirements.in for `uv pip compile`
  Exact prior report: inline `--config-settings` produced `Expected --hash`. A maintainer confirmed requirements files do not support it and redirected the report to astral-sh/uv#3636; it was closed as a duplicate.
- https://github.com/astral-sh/uv/pull/14573 (merged pull request): Allow `--config-settings-package` to apply configuration settings at the package level
  Adjacent capability, not a requirements-file fix: astral-sh/uv#14573 added package-scoped configuration through uv's CLI/configuration, but did not add pip-compatible `--config-settings` parsing to requirements files.
