# Azure Artifacts authentication via artifacts-keyring fails with `UV_KEYRING_PROVIDER=subprocess`

Issue: astral-sh/uv#23362

Classification: question

## Summary

On Windows with uv 0.13.9, the reporter installed `keyring` with `artifacts-keyring` as a uv tool, set the documented Azure index username, and enabled the subprocess provider. Running keyring through `uv tool run` finds the backend and reportedly succeeds after browser sign-in, but `uv pip install --dry-run -vv` receives `401 Unauthorized`. The trace shows lookup of the configured index URL followed by fallback to the host. The closest confirmed precedent is a different `keyring` executable taking precedence on `PATH`; that has not been confirmed here.

## Draft response

Your trace shows that uv tries the configured index URL and falls back to the host. `uv pip install` launches `keyring` by name, so a successful `uv tool run keyring` check doesn't tell us which executable the installer uses. That distinction mattered in astral-sh/uv#11130, where another Windows installation of `keyring` took precedence.

In the same Command Prompt where the install fails, run `where.exe keyring` and `keyring --list-backends`. Check that the first result is in the directory reported by `uv tool dir --bin` and that the backend list includes `ArtifactsKeyringBackend`. If the tool directory is missing or another keyring comes first, put the tool directory first on `PATH` and retry. If those checks pass and the 401 persists, please confirm that the direct check used the exact index URL shown in the trace and share any redacted keyring warnings. Please do not share credentials or the output of `keyring get`.

## Classification

This is an authentication troubleshooting report, not a confirmed uv bug or regression. The trace confirms the setting is active and the lookup ran. It does not identify the executable that ran, and the direct check through the tool runner does not verify the documented `PATH` prerequisite. The historical Windows report establishes that executable selection can cause this symptom, not that it caused this instance. There is also no before/after uv version comparison in the report.

## Related

- astral-sh/uv#11130 (closed) — Closest precedent: Windows, `artifacts-keyring`, index lookup, host fallback, and a 401. The reporter confirmed that Anaconda's `keyring` lacked the plugin and correcting `PATH` resolved it. This report has not checked bare `keyring` from the failing shell.
- astral-sh/uv#12042 (open) — Another Azure Artifacts subprocess failure where commenters observed an active virtual environment's plugin-less `keyring` shadowing the uv-installed tool. The new report does not identify an active virtual environment or show that one shadows the tool.
- astral-sh/uv#17357 (open) — Another Azure cloud feed 401 with keyring, but that report describes a configured URL versus a UUID-form URL and success with `--extra-index-url`. This trace shows lookup of the configured index URL, and the successful comparison is a direct tool invocation, not another index option.
- astral-sh/uv#12651 (merged) — Shipped in 0.7.0 and fixed package-URL lookups for backends that require the index root, closing astral-sh/uv#4583, astral-sh/uv#11236, astral-sh/uv#11391, and astral-sh/uv#11507. The new trace already shows the index-root lookup, so it does not show a recurrence of that bug.

## Supporting evidence

- `docs/guides/integration/azure.md:41` documents the Azure plugin setup and requires the `keyring` executable to be on `PATH`. `docs/concepts/tools.md:254` explains how the tool executable directory gets onto `PATH`.
- `crates/uv-auth/src/middleware.rs:853` uses the configured index URL when it has both a username and index context. `crates/uv-auth/src/keyring.rs:207` tries that URL first, then the host if the first lookup returns no credentials to uv. `crates/uv-auth/src/keyring.rs:262` invokes `keyring get` by name rather than calling the tool runner.
- `CHANGELOG.md:6` records uv 0.13.9, released November 17, 2026. Neither the report nor the related fixes establish a change in this behavior in that release.

## Search Scope

Searched open and closed issues and open, closed, and merged pull requests with authenticated `gh`. Literal searches covered `401 Unauthorized`, `artifacts-keyring`, `UV_KEYRING_PROVIDER`, `VssSessionToken`, `Checking keyring for host`, and `uv pip install`. Broader searches covered direct keyring versus installer authentication, index-root lookups, executable selection and `PATH`, and subprocess output. Checked issue comments, cross-references, merged commits, and release notes for version-specific fixes. Ruled out astral-sh/uv#18611 (provider lookup never ran), astral-sh/uv#17144 (on-premises Azure DevOps Server), astral-sh/uv#3260 (first-attempt-only failure with plugin output mixed into the credential), and astral-sh/uv#17436 / astral-sh/uv#17448 (tool `@latest` lookup skipped keyring).
