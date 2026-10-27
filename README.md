# `pyproject.toml` parse warnings should be an error instead

Issue: astral-sh/uv#22951

Classification: duplicate

## Summary

The reporter shows `uv sync` warning about an unknown `asdf = true` setting in `pyproject.toml`, requests an error/nonzero exit by default or through a strict option, and raises concern that other settings such as `index-url` and `exclude-newer` are discarded. They suggest version skew as a possible motivation for the current behavior and link discussions about project-local uv installation and promoting package-conflict warnings to errors. No complete reproduction or uv version is provided in this issue.

astral-sh/uv#18922 and its open draft astral-sh/uv#18923 are the closest matches. astral-sh/uv#17609 and astral-sh/uv#18977 document the narrower, already-fixed required-version case; astral-sh/uv#9911 covers duplicate parse diagnostics.

Current source confirms that settings discovery skips a `pyproject.toml` when settings deserialization fails. This can discard valid settings alongside the invalid one. It does not mean all configuration sources stop loading: discovery can continue to parent directories, and user/system configuration is loaded separately. The version-skew explanation is the reporter's hypothesis, not an established rationale for the general policy.

## Draft response

You're right that a settings parse failure can discard otherwise-valid settings from the same `pyproject.toml`. This is the underlying behavior tracked in astral-sh/uv#18922. The targeted proposal in astral-sh/uv#18923 is still an open draft, and making these failures fatal needs a compatibility decision. Please add your unknown-field example and request for broader validation to astral-sh/uv#18922 so we can keep the discussion together.

## Classification

astral-sh/uv#18922 already tracks the underlying correctness problem: settings parse failures become warnings, configuration is discarded, and execution continues. The unknown-field example and request for broader or opt-in fatal validation can be discussed there, with the existing draft astral-sh/uv#18923. Duplicate classification takes precedence. The merged astral-sh/uv#18977 only fixed required-version enforcement, so this is not evidence that a general parse-error fix regressed.

The existing enhancement label on the new issue does not change this recommendation. The correctness concern is supported by source and by the maintainer's response on astral-sh/uv#18922; an open discussion already covers it. The broader unknown-field example does not require a separate canonical thread. Neither of the historical merged fixes inspected made arbitrary settings errors fatal.

## Related

- astral-sh/uv#18922 — `exclude-newer`: Parsing failures should become an error (issue, open). Closest canonical discussion: uv sync continues after a settings-discovery parse warning and ignores exclude-newer. A maintainer confirmed this was unintended and noted that making it fatal is breaking. The new report adds unknown fields as another trigger for the same configuration-discarding behavior.
- astral-sh/uv#18923 — Fail on `exclude-newer` parse errors (pull request, open). Existing draft targeting astral-sh/uv#18922, opened months before this report. Its description explicitly discusses reconsidering settings parse failures more broadly, but limits the proposal to exclude-newer. The author deferred merging it in July 2026; it is not a shipped fix.
- astral-sh/uv#17609 — tool.uv.required-version should be handled before parsing rest of pyproject.toml file (issue, closed). Historical instance of the same parse failure discarding another setting: unsupported configuration bypassed required-version. This narrower version-enforcement bug was fixed; the new report does not establish a regression of that fix.
- astral-sh/uv#18977 — Elevate configuration errors to `required-version` mismatches (pull request, merged). Merged April 13, 2026, fixing astral-sh/uv#17609 by checking required-version after settings parsing fails. Current source retains this check but still warns and skips the file when no version mismatch is found; it does not implement general fatal validation.
- astral-sh/uv#9911 — `pyproject.toml` TOML parse error/warning shown twice (issue, open). Adjacent settings-discovery discussion whose comments propose making parse failures fatal. Its reproduction has invalid TOML syntax and already ends in an error; this report concerns schema-invalid settings that can allow execution to succeed.

## Supporting evidence

- `crates/uv-settings/src/lib.rs:106`, `FilesystemOptions::find`: `Error::PyprojectToml` emits the exact settings-discovery warning and continues searching ancestors instead of returning an error.
- `crates/uv-settings/src/lib.rs:134`, `FilesystemOptions::from_directory`: deserializes the file's settings as a whole; a deserialization error prevents those settings from being returned. Errors from `uv.toml` take the fatal path instead.
- `crates/uv/src/lib.rs:339`: project settings are combined with separately loaded user and system settings. The confirmed claim is loss of settings from the affected file, not loss of every configuration source.
- `crates/uv-settings/src/lib.rs:265`, `check_pyproject_required_version`: after a settings parse failure, a separate parse checks the version requirement and elevates a mismatch to an error. Without a mismatch, the original parse error follows the warning path.
- `crates/uv/tests/pip_install/pip_install.rs:851` and `:881`: existing tests snapshot exit code 0 for an invalid `index-url` type and an unknown configuration field respectively.
- `crates/uv/tests/pip_install/pip_install.rs:917`: the existing required-version regression test snapshots exit code 2 for an incompatible version requirement alongside an unknown field.
- `crates/uv/tests/lock_scenarios/lock_exclude_newer_relative.rs:1287`: existing lock tests snapshot warnings and successful exits for invalid `exclude-newer` values.
- In astral-sh/uv#18922, a maintainer wrote “That's not intended” on April 8, 2026, followed by a warning that changing it would be breaking. Its October 5 comment reports continuing warning-only behavior on uv 0.12.16, while distinguishing the required-version fix. Those version observations are commenter reports; they were not independently reproduced.
- astral-sh/uv#18923 is an open draft labeled bug and breaking. Its author wrote on July 28, 2026, “It's unclear that we should merge this still. I will defer.” Its proposal only validates `exclude-newer` more aggressively, so it does not yet address every unknown-field trigger.
- astral-sh/uv#18977 merged on April 13, 2026, before this October 27 report. Its description and retained source limit the fix to required-version mismatches. astral-sh/uv#6009, merged in August 2024, only added the underlying parser error to the warning.

Evidence comes from read-only GitHub inspection, current source, and checked-in snapshots. No runtime reproduction or test execution was performed, and no exploit or concrete security incident is established by this report.

## Other candidates assessed

- astral-sh/uv#5934 and astral-sh/uv#6009: nearly identical unknown-field warning, but the request and fix were to show the parser's explanation, not change exit status. Their closure is not evidence of a regression here.
- astral-sh/uv#9653: related CI validation goal through a separate `uv check`-style command; it does not track changing settings discovery during `uv sync`.
- astral-sh/uv#6662: reporter-supplied lead about installing and pinning uv through a project wrapper; useful version-management context, but a different capability.
- astral-sh/uv#16546: reporter-supplied lead about conflicting installed package files. Its warning/error debate has a different subsystem and compatibility constraint.
- astral-sh/uv#22950: uses the same `asdf` warning as a probe, but primarily questions `preview` versus `preview-features` semantics.
- astral-sh/uv#17308: unknown `python-version` setting with workspace-dependent discovery; it asks about that configuration/discovery behavior rather than general fatal validation.
- astral-sh/uv#3097: strict mode means restricting `uv run` to executables in the environment, not treating warnings as errors.

## Search coverage

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered settings discovery, unknown field, pyproject.toml parse warnings, exclude-newer, and required-version. Conceptual searches covered ignored configuration, invalid settings, fatal parsing, strict mode, warnings as errors, and CI configuration validation, without incidental package/version/platform restrictions. Fix-oriented searches and issue timelines led to astral-sh/uv#18923, astral-sh/uv#18977, and astral-sh/uv#6009; inspected their descriptions, comments, and reviews alongside current source and existing snapshots. PR keyword searches returned no hits, so linked PRs were inspected directly. Ruled out astral-sh/uv#5934 and its merged fix astral-sh/uv#6009 as diagnostic-detail improvements rather than fatal-validation fixes; astral-sh/uv#9653 requests a separate validation command; astral-sh/uv#3097 restricts executable lookup. Reporter links astral-sh/uv#6662 and astral-sh/uv#16546 concern uv bootstrapping and package-file conflicts respectively. Also inspected astral-sh/uv#22950 and astral-sh/uv#17308: preview semantics and workspace-dependent discovery are not the canonical discussion for this request.

## Maintainer next step

Centralize the unknown-field example and strict-validation request in astral-sh/uv#18922. The existing draft astral-sh/uv#18923 documents the narrower proposal and unresolved compatibility decision. Avoid presenting that draft or the required-version fix as a shipped solution to arbitrary invalid settings.
