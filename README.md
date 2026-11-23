# Azure Artifacts authentication via artifacts-keyring fails with `UV_KEYRING_PROVIDER=subprocess`

Issue: astral-sh/uv#23362

Classification: question

## Summary

On Windows with uv 0.13.9, the reporter installed `keyring` with `artifacts-keyring` as a uv tool, configured a named private Azure Artifacts index and default PyPI in `uv.toml`, and set `UV_INDEX_COMPANY_PYTHON_FEED_USERNAME=VssSessionToken` and `UV_KEYRING_PROVIDER=subprocess`. They expected `uv pip install --dry-run package-name -vv` to use the same backend that reportedly succeeds after browser sign-in through `uv tool run keyring get`. Instead, the installer traces an index-root lookup and host fallback, then reports `401 Unauthorized`. The report does not give the Python, `keyring`, or `artifacts-keyring` versions or identify the bare `keyring` executable visible to the failing shell.

## Classification

Authentication troubleshooting, not a confirmed uv bug or regression. The controlled reproduction shows that `PATH` shadowing is sufficient to produce the reported discrepancy, but does not establish that it occurred on the reporter's machine. No before/after uv version comparison is reported.

## Reproduction

**Outcome: `reproducible` with a shadowing `keyring` in a controlled fixture.** Used uv 0.13.9 from `PATH`, Linux x86_64, CPython 3.12.3, `keyring` 25.7.0, and `artifacts-keyring` 1.0.0. A temporary `uv tool install keyring --with artifacts-keyring` environment used the real keyring CLI and Azure backend, with the bundled credential-provider executable replaced by a stand-in. A loopback HTTPS fixture served the Azure-shaped Simple API and synthetic package, with a local 404 response for the PyPI fallback. No live feed, Windows sign-in, session cache, or real credentials were used.

The fixture used `[[index]] name = "company-python-feed"` with an Azure-shaped `/uv-triage/_packaging/example/pypi/simple/` URL, and a second, default `pypi` index. It installed a second copy of `keyring` without the plugin into a separate bin directory ahead of the plugin-enabled tool on `PATH`. The tool install and tool run used matching provider settings so that the tool runner reused its installed environment. With `$FEED` set to the fixture index root and `$TARGET_PYTHON` pointing to a separate temporary virtual environment, the relevant commands were:

```sh
export UV_KEYRING_PROVIDER=subprocess
export UV_INDEX_COMPANY_PYTHON_FEED_USERNAME=VssSessionToken
uv tool run keyring --list-backends
keyring --list-backends
uv tool run keyring get "$FEED" VssSessionToken >/dev/null
uv pip install --dry-run uv-triage-synthetic -vv --no-cache --python "$TARGET_PYTHON"
```

- **Plugin-less keyring first:** `uv tool run keyring --list-backends` listed the Azure backend and `uv tool run keyring get` exited 0. Bare `keyring --list-backends` did not list the backend. The dry run exited 1 after the index-root lookup and host fallback, with a `401 Unauthorized` hint.
- **Plugin-enabled keyring first:** Changing only the `PATH` order made the same dry run find keyring credentials at the index root, resolve `uv-triage-synthetic==1.0.0`, and exit 0.
- **Provider disabled:** With the plugin-enabled keyring still first, setting `UV_KEYRING_PROVIDER=disabled` restored the 401 without the keyring lookup messages.

Existing tests cover the underlying auth paths, not the Windows/Azure report. `crates/uv/tests/pip_install/pip_install.rs:7989` (`install_package_basic_auth_from_keyring`) puts the test keyring on `PATH`, asserts index-root then host lookup, and installs successfully with a host entry. `crates/uv/tests/pip_install/pip_install.rs:8085` (`install_package_basic_auth_from_keyring_wrong_password`) checks the same lookup sequence and a 401 with a bad host entry. `crates/uv/tests/project/edit.rs:13537` (`add_index_url_in_keyring`) exercises a named index username environment variable and a successful lookup at the index root. The tests were read, not rerun.

## Suggested follow-up

In the same Windows Command Prompt where installation fails, compare `where.exe keyring` with `uv tool dir --bin` and run bare `keyring --list-backends`. If an executable without `ArtifactsKeyringBackend` takes precedence, put the plugin-enabled tool directory first on `PATH` and retry. If bare keyring lists the backend and the 401 persists, ask for the Python and package versions, the redacted executable paths, and any redacted subprocess warnings. Confirm that the direct check uses the exact index root from the installer trace. Do not request the output of `keyring get` or any credentials.

## Related

- astral-sh/uv#11130 — Closest precedent: Windows, `artifacts-keyring`, index lookup, host fallback, and a 401. The reporter confirmed that Anaconda's `keyring` lacked the plugin and correcting `PATH` resolved it. The current report has not identified which bare `keyring` is visible in the failing shell.
- astral-sh/uv#12042 — Commenters observed an active virtual environment's plugin-less `keyring` shadowing the uv-installed tool. The current report does not identify an active virtual environment or show that one shadows the tool.
- astral-sh/uv#17357 — Another Azure cloud feed 401, but that report describes a configured URL versus a UUID-form URL and success with `--extra-index-url`. The current trace shows lookup of the configured index URL, and the successful comparison is a direct tool invocation, not another index option.
- astral-sh/uv#12651 (merged) — Shipped in 0.7.0 and fixed package-URL lookups for backends that require the index root, closing astral-sh/uv#4583, astral-sh/uv#11236, astral-sh/uv#11391, and astral-sh/uv#11507. The new trace already shows the index-root lookup, so it does not show a recurrence of that bug.

## Supporting evidence

- `docs/guides/integration/azure.md:55` requires the plugin-enabled `keyring` executable to be on `PATH`. `docs/concepts/tools.md:254` explains how the tool executable directory gets onto `PATH`.
- `crates/uv-auth/src/middleware.rs:856` selects the configured index URL when a username and index context are available. `crates/uv-auth/src/keyring.rs:227` falls back to the host when the initial lookup returns no credentials. `crates/uv-auth/src/keyring.rs:268` invokes `keyring get` by name, while `crates/uv-tool-commands/src/run.rs:430` prepends the chosen tool environment's scripts directory when launching a tool.
- `CHANGELOG.md:6` records uv 0.13.9, released November 17, 2026. Neither the report nor the related fixes establish a change in this behavior in that release.

## Search Scope

Prior triage searched issue comments, cross-references, merged changes, and release notes for Azure authentication failures, keyring subprocess behavior, and executable selection. Less similar cases include astral-sh/uv#18611 (provider lookup never ran), astral-sh/uv#17144 (on-premises Azure DevOps Server), astral-sh/uv#3260 (first-attempt-only failure with plugin output mixed into the credential), and astral-sh/uv#17436 and astral-sh/uv#17448 (tool `@latest` lookup skipped keyring). This reproduction checked the current auth and tool-run implementations and the relevant pip, project, and client integration test modules.
