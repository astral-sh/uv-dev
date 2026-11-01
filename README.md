# uv audit does not respect UV_PYTHON and missing --python flag

Issue: astral-sh/uv#23034

Classification: bug

## Summary

A focused fix is implemented in the checkout: audit now honors `--python` and `UV_PYTHON`, and applies explicit Python/platform targets when selecting dependencies. All five regressions from astral-sh/uv-dev#3059 now assert the corrected behavior. Two additional integration tests cover script interpreter discovery and project-less workspace group requirements. The 49-test audit module and an existing tool-install/audit round trip pass in the debug profile. See **Fix** for implementation and validation details.

The behavior reported in astral-sh/uv#23034 is **reproducible** with the installed uv 0.13.5 on Ubuntu 24.04.5, Linux x86_64. The report used uv 0.13.6 on Linux aarch64 with Python 3.13.18; that exact binary/container was not run. The reproduction used managed CPython 3.13.17 and downloaded CPython 3.11.17.

With `UV_PYTHON=3.13` and `.python-version` set to 3.11, `uv lock` uses Python 3.13, but `uv audit` selects and downloads Python 3.11. Audit rejects `--python 3.13`. Plain `--frozen` avoids interpreter discovery; adding `--python-version 3.13` restores discovery and the 3.11 download.

The target-filtering claim also reproduces. A Python-version-gated dependency is excluded by `uv tree` for Python 3.13 but remains audited with `uv audit --frozen --python-version 3.13`, including after repinning the interpreter to 3.13. A separate Windows-only dependency fixture likewise remains audited with an explicit Linux target, while `uv tree` excludes it.

No exact duplicate was found in the prior issue/history search. astral-sh/uv#18506 and astral-sh/uv#18185 explain interpreter downloads and the frozen workaround; astral-sh/uv#18511 provides related subset-filtering history.

## Draft response

The reported behavior was reproduced, and the checkout now includes a fix. Audit accepts `--python` and honors `UV_PYTHON` when discovering an interpreter for either a project or a script. Explicit Python-version and platform targets filter the audited dependency graph, including requirements stored directly in script and project-less workspace lock manifests.

Plain `--frozen` without either target flag still avoids interpreter discovery and performs a universal audit. A target flag still requires an interpreter for the remaining environment markers, but an explicit interpreter request overrides the version-file pin. Interpreter selection and audit-target filtering remain separate concerns.

## Classification

Pre-fix commands confirmed the interpreter-selection omission, rejected --python option, and ineffective target filters. The baseline implementation supplied no explicit interpreter request, exposed no --python/UV_PYTHON binding, and computed but discarded the markers from --python-version and --python-platform. The fix connects those inputs to discovery and dependency traversal. The pre-fix target-filtering behavior contradicted the documentation, independently of whether adding --python is viewed as an enhancement. Historical PRs concern universal defaults and group/extra filtering; no previously fixed target-filtering bug or exact duplicate was established.

An interpreter download during a normal audit is not by itself a bug: audit may update or validate the lockfile. Selecting the interpreter used for locking is also distinct from selecting the target environment whose dependencies should be audited. The confirmed correctness problem was that explicit target flags promised filtering but did not apply it. The missing interpreter override was a separate omission; both are corrected in the checkout.

There is no evidence that a previously released fix for either omission has regressed. The historical universal/frozen condition predates the reported version. The dependency-group fix discussed below addressed a different filter.

## Reproduction

**Outcome: reproducible.** Commands used `/opt/hostedtoolcache/uv/0.13.5/x86_64/uv`, the executable already on `PATH`, reporting `uv 0.13.5 (x86_64-unknown-linux-gnu)`. No checkout binary was built. The host is Ubuntu 24.04.5 LTS, Linux x86_64. The reporter's uv 0.13.6/aarch64 container and Python 3.13.18 were not tested, so this does not establish an exact-version or architecture comparison. The issue does not identify a last known-good release.

All fixtures, Python installations, configuration/data locations, and caches were isolated under `/tmp/uv-23034-l7la8aaw`. Subprocesses used a clean environment with `UV_PYTHON=3.13`, `UV_PYTHON_PREFERENCE=only-managed`, and temporary `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR`, `UV_PYTHON_BIN_DIR`, XDG directories, home, and temporary-file directory. Managed-only discovery models the report's missing-3.11 container without using host interpreters. Configuration discovery remained enabled so `.python-version` was respected; `--no-config` would suppress that file. Each download scenario started with only CPython 3.13.17 available in its temporary managed-install directory. No workspace members, extras, dependency groups, or existing virtual environment were involved.

### Interpreter selection and missing option

In an otherwise empty temporary project, install Python 3.13 into the isolated installation directory and use this configuration:

```toml
[project]
name = "repro"
version = "0.1.0"
requires-python = ">=3.11"
```

Set `.python-version` to `3.11`, with `UV_PYTHON=3.13` for every invocation:

```sh
uv python install 3.13
uv lock -v
uv audit -v --preview-features audit-command --frozen
uv audit --preview-features audit-command --python 3.13
uv audit -v --preview-features audit-command
```

Observed results:

- Installation provided CPython 3.13.17. `uv lock -v` exited 0 and logged ``Using Python request `3.13` from explicit request`` and `Using CPython 3.13.17`.
- Plain frozen audit exited 0 and reported zero auditable packages, with no interpreter lookup or download in the debug log.
- `--python 3.13` exited 2 with `error: unexpected argument '--python' found` and the suggestion `--python-fetch`.
- Non-frozen audit exited 0, logged ``Using Python request `3.11` from version file at `.python-version` ``, downloaded `cpython-3.11.17-linux-x86_64-gnu` (29.1 MiB), and printed `Using CPython 3.11.17`.

Raw output and exit codes are in `minimal-lock`, `minimal-frozen`, `missing-python-option`, and `minimal-audit` `.log`/`.exit` files beneath the reproduction directory.

### Python target and frozen behavior

A separate project used the same project metadata plus:

```toml
dependencies = ["requests==2.32.5; python_version < '3.12'"]
```

Its version file initially contained `3.11`; `UV_PYTHON` remained `3.13`. A separate temporary managed-install directory initially exposed only Python 3.13.17. Run:

```sh
uv lock
uv tree --frozen --python-version 3.11 --depth 1
uv tree --frozen --python-version 3.13 --depth 1
uv audit -v --preview-features audit-command --frozen
uv audit -v --preview-features audit-command --frozen --python-version 3.13
printf '3.13\n' > .python-version
uv audit -v --preview-features audit-command --frozen --python-version 3.13
```

`uv lock` used Python 3.13.17 and resolved six packages including the root. Both tree commands exited 0: the 3.11 tree included `requests v2.32.5`, while the 3.13 tree contained only `repro v0.1.0`.

All three audits exited 1 and reported exactly:

```text
Found 2 known vulnerabilities and no adverse project statuses in 5 packages
```

The findings were for `requests 2.32.5`. The audited dependency set comprised requests, certifi, charset-normalizer, idna, and urllib3. Plain frozen audit performed no interpreter discovery. Adding `--python-version 3.13` selected and downloaded CPython 3.11.17 from the version-file request, despite `UV_PYTHON=3.13`. After repinning `.python-version` to 3.13, audit used CPython 3.13.17 but still audited the same five packages and reported the same findings. Thus the filtering failure persists independently of the interpreter mismatch. Advisory counts are service-dependent; the decisive observation is inclusion of dependencies that the target excludes.

Raw output and exit codes are in the `markers-*` `.log`/`.exit` files beneath the reproduction directory.

### Platform target

A third isolated project used the same metadata, a 3.11 version-file pin, `UV_PYTHON=3.13`, and:

```toml
dependencies = ["requests==2.32.5; sys_platform == 'win32'"]
```

```sh
uv lock
uv tree --frozen --python-platform x86_64-pc-windows-msvc --depth 1
uv tree --frozen --python-platform x86_64-unknown-linux-gnu --depth 1
uv audit -v --preview-features audit-command --frozen --python-platform x86_64-unknown-linux-gnu
```

The Windows tree included requests; the Linux tree contained only the root. Audit for Linux nevertheless selected the already-installed CPython 3.11.17, audited five packages, and reported the same two vulnerability findings (exit 1). This directly confirms both interpreter discovery with a frozen platform target and ineffective platform filtering. A fresh platform-triggered Python download was not separately tested. Raw output is in the `platform-*` `.log`/`.exit` files.

### Existing test coverage

At the initial reproduction baseline, searches of `crates/uv/tests/it/`, `crates/uv-client/tests/it/`, and `crates/uv/tests/build/audit.rs` found no direct audit coverage of `UV_PYTHON`, `--python-version`, or `--python-platform`. The parent regression PR subsequently added five cases; those are retained and corrected by this fix. These nearby tests were read through their setup and assertions:

- `audit_no_dependencies` locks an empty project and snapshots a successful audit of zero packages; it has no conflicting interpreter requests.
- `audit_json_no_vulnerabilities` uses a one-package lockfile and mock audit service, then checks frozen JSON output with one audited package. It does not test interpreter discovery or target filtering.
- `audit_dependency_groups` checks all-groups, no-dev, no-group, and only-group package counts. `audit_no_default_groups` checks three packages by default versus one with the flag, despite `default-groups = ["dev"]`. Neither exercises environment markers.

The audit module is gated by both `test-python` and `test-pypi` in `crates/uv/tests/build/main.rs`. The initial reproduction used the installed CLI without changing checkout files or GitHub state. Subsequent debug-profile regression validation is recorded under **Fix**.

## Related

- astral-sh/uv-dev#3059 — Parent regression pull request. Adds five integration tests in `crates/uv/tests/build/audit.rs` asserting the reported undesirable behavior; all five are updated by this fix.
- astral-sh/uv#18506 — Roadmap: `uv audit` (issue, open). Maintainer comments explain that audit normally locks first and recommend --frozen to avoid interpreter downloads. This explains the workaround, but does not track ignored UV_PYTHON or ineffective target filters.
- astral-sh/uv#18185 — Make universal the default for `uv audit` (pull request, merged). Introduced the exact condition that skips interpreter discovery only for frozen audits without --python-version or --python-platform. It establishes the historical behavior, not a fix for interpreter overrides or target filtering.
- astral-sh/uv#18511 — Evaluate extras and groups when determining auditable packages (pull request, merged). Introduced lockfile traversal filtered by extras and dependency groups. Maintainer discussion supports auditing everything by default with explicit subset selection; the implementation does not supply Python/platform markers to that traversal.

## Supporting evidence

The following source evidence describes the pre-fix baseline at commit `564a2074b02eb9de5aaaa9491ed74357ade0332f`. The production changes described under **Fix** correct these missing connections; the line numbers below refer to that baseline.

- `crates/uv-cli/src/lib.rs:5056`: `AuditArgs` exposes target version/platform options, but no interpreter request or `UV_PYTHON` binding. Its target-option documentation explicitly promises dependencies for the requested environment.
- `crates/uv-cli/src/settings.rs:3182`: `AuditSettings` carries `python_version` and `python_platform`, with no interpreter override.
- `crates/uv-project-commands/src/audit.rs:108`: universal mode means neither target option is present. Interpreter discovery is skipped only when both frozen and universal are true.
- `crates/uv-project-commands/src/audit.rs:133`: project discovery receives `None` as the explicit Python request. Script discovery likewise receives `None`.
- `crates/uv-python-discovery/src/project.rs:122`: without an explicit request, discovery consults `.python-version` before falling back to the project's Python requirement. This explains the supplied 3.11 versus 3.13 example.
- `crates/uv-project-commands/src/audit.rs:198`: `resolution_markers` computes `_markers`, which is not passed to `audit_lock`.
- `crates/uv-audit-operations/src/lib.rs:41`: `audit_lock` accepts no target markers and selects packages with `lock.auditable(extras, groups, |_| true)`.
- `crates/uv-lock/src/lock/mod.rs:3506`: audit traversal accepts extras, groups, and a package collection predicate, with no target marker environment. These findings confirm why target flags do not filter the package selection.
- In astral-sh/uv#18506, maintainer comments dated April 9, 2026 explain the locking step and recommend `--frozen` to avoid interpreter installation. That advice describes the universal path.
- The diff for astral-sh/uv#18185, merged February 24, 2026, introduces the same universal/frozen interpreter-discovery condition and retains the documented target flags.
- The diff for astral-sh/uv#18511, merged March 26, 2026, introduces `packages_for_audit` with extras/groups inputs and no target markers.
- `CHANGELOG.md:6` dates uv 0.13.6 to October 26, 2026. The report was opened November 1, 2026. The relevant historical changes therefore precede the reported release.

## Other candidates inspected

- astral-sh/uv#18968 initially resembles a container/platform-specific audit discrepancy, but its comments narrow the problem to `UV_CONFIG_FILE` suppressing project vulnerability-ignore configuration. It is not interpreter selection or marker filtering.
- astral-sh/uv#22089 reports a documented audit flag doing nothing, but specifically concerns `--no-default-groups`. Its merged fix, astral-sh/uv#22090 on September 30, 2026, changes group selection and adds a group-selection regression test. It does not implement Python/platform filtering.
- astral-sh/uv#18970, merged April 12, 2026, fixes script handling and collecting extras during audit traversal. Neither fix addresses this report.
- astral-sh/uv#18119 provides the original audit scaffolding and leads to the universal-default work and roadmap.
- astral-sh/uv#22252 and astral-sh/uv#22192, both merged October 7, 2026, move shared audit workflows into modules/crates. Their descriptions establish refactoring context, not a prior fix for target filtering.

## Search coverage and limits

Searched astral-sh/uv open and closed issues and all PR states using audit, UV_PYTHON, --python, python-version, python-platform, frozen, and unexpected argument; separately searched interpreter/download behavior and marker/filtering concepts without package or platform restrictions. Reviewed audit-area issues, comments, referenced PRs, historical diffs, and audit-titled entries from the latest 4,000 PRs. Ruled out astral-sh/uv#18968 (UV_CONFIG_FILE precedence), astral-sh/uv#22089 and its merged fix astral-sh/uv#22090 (dependency-group selection), and astral-sh/uv#18970 (script handling and extras traversal). Additional targeted searches hit GitHub's rate limit; no exact duplicate or previous fix for these gaps was found in the retrieved evidence.

The initial decomposition kept observable behavior separate from implementation hypotheses: interpreter precedence and rejected flags; frozen-mode downloads; and target-specific dependency selection. Literal searches included the option/environment names and the unexpected-argument fragment. Conceptual searches covered interpreter discovery/downloads, universal audits, markers, filtering, and subset selection. Closed issues, merged changes, roadmap comments, and referenced fixes were checked for regression history.

Some PR search queries returned no results despite known PRs being accessible directly. To supplement those searches, the latest 4,000 PRs across all states were listed and audit-related titles reviewed, spanning the original audit scaffolding through current changes. Later targeted searches returned rate-limit errors. The duplicate assessment is based on the retrieved evidence, not a claim of exhaustive search coverage.

## Fix

**Outcome: fixed in the checkout.** No commits, pushes, Git configuration changes, or GitHub mutations were made. Changes are limited to affected production Rust files and `crates/uv/tests/build/audit.rs`; this README is the only modified issue-context file.

### Confirmed cause and implementation

The CLI/settings/dispatch path did not provide an interpreter request to audit. Both project and script discovery received `None`, allowing `.python-version` to take precedence regardless of `UV_PYTHON`. Audit now exposes the standard `--python`/`-p` option with `UV_PYTHON` binding, resolves it through settings, and forwards the request to both discovery implementations. The CLI option overrides the environment value and project pin, matching neighboring project commands.

Audit computed target markers but discarded them, and the auditable lockfile traversal had no target environment. It now accepts an optional marker environment and filters dependency edges before enqueueing their packages/extras. Script requirements and project-less workspace group requirements are filtered using their requirement markers intersected with the package's resolution forks. This excludes inapplicable packages and their transitive dependencies, and selects the correct version when a universal lock contains multiple versions of the same package.

The shared audit operation consumes the selected `Auditable` graph, so vulnerability queries and project-status queries use the same selection without introducing crate dependencies. Tool audit and malware-check callers continue to request universal traversal. An audit without either target flag remains universal; plain frozen audit continues to skip interpreter discovery. An explicit target still uses discovery for the other environment markers, while honoring the requested interpreter.

### Regression evidence and scope

Before production changes, the four parent `audit_python*` tests and `audit_script_python_version` all passed while asserting the bug. After updating their expected behavior, all five failed for the reported reasons: missing Python 3.11 despite the 3.13 override, rejected `--python`, and one audited package where the target should exclude it.

All five parent tests remain in `crates/uv/tests/build/audit.rs`:

- `audit_python_request` checks the environment override with normal audit, plain frozen audit, and a frozen explicit target.
- `audit_python_argument` now uses a minimal project and verifies the CLI request overrides both the environment and version-file pin.
- `audit_python_version` excludes the conditional dependency for Python 3.13 and includes it for a Python 3.11 target while continuing to use the Python 3.13 interpreter.
- `audit_python_platform` excludes a Windows-only dependency for Linux.
- `audit_script_python_version` excludes a conditional requirement stored in the script lock manifest.

Two distinct manifestations were demonstrated before fixing production code and are covered by separate tests:

- `audit_script_python_request`: the separate script-discovery implementation also ignored `UV_PYTHON`. Before the fix it requested unavailable Python 3.11; after the fix, a script lock/audit round trip succeeds with Python 3.13.
- `audit_python_platform_workspace_groups`: the separate manifest dependency-group traversal audited three packages for Linux, including a Windows-only requirement and both platform-specific versions of iniconfig. It now audits only iniconfig 2.0.0; the mock service checks the exact selected package/version.

The audit module's existing `test-python`/`test-pypi` gates are retained, and the new platform-marker case also uses `test-universal`. Existing extras/group, script, JSON, SARIF, vulnerability, and project-status tests pass without snapshot changes. Related tree fixtures in `crates/uv/tests/project/tree.rs` already represent target-aware selection; no contradictory assumptions required edits there. The existing `crates/uv/tests/tool/tool_audit.rs::tool_audit_one_tool` fixture verifies that installing a tool with a lockfile and auditing that lockfile still works through the shared audit operation. No speculative command variants or unrelated fixtures were added.

### Successful focused validation

- `cargo test -p uv --test build --features test-python,test-pypi,test-universal audit::` — 49 passed in the debug/test profile, including all updated parent regressions and both additional cases.
- `cargo test -p uv --test tool --features test-python,test-pypi,test-universal tool_audit::tool_audit_one_tool -- --exact` — 1 passed; tool installation produces the lockfile consumed by audit.
- `RUSTUP_TOOLCHAIN=stable cargo fmt --all` and `RUSTUP_TOOLCHAIN=stable cargo fmt --all -- --check` — passed. The installed stable rustfmt was used because the pinned 1.99.0 toolchain lacks rustfmt and its installation directory is read-only.
- `git diff --check` — passed; changed checkout paths are restricted to the permitted production and integration-test files.

The full repository test suite and release-profile builds were not run. The initial installed-binary reproduction used uv 0.13.5; fix validation builds the current uv 0.13.6 checkout in the debug profile. Advisory-service counts from the initial reproduction remain service-dependent; the regression tests use mock audit responses and verify dependency selection directly.

Pull request: https://github.com/astral-sh/uv-dev/pull/3060
