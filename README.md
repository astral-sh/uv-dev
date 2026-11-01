# uv audit does not respect UV_PYTHON and missing --python flag

Issue: astral-sh/uv#23034

Classification: bug

## Summary

The behavior reported in astral-sh/uv#23034 is **reproducible** with the installed uv 0.13.5 on Ubuntu 24.04.5, Linux x86_64. The report used uv 0.13.6 on Linux aarch64 with Python 3.13.18; that exact binary/container was not run. The reproduction used managed CPython 3.13.17 and downloaded CPython 3.11.17.

With `UV_PYTHON=3.13` and `.python-version` set to 3.11, `uv lock` uses Python 3.13, but `uv audit` selects and downloads Python 3.11. Audit rejects `--python 3.13`. Plain `--frozen` avoids interpreter discovery; adding `--python-version 3.13` restores discovery and the 3.11 download.

The target-filtering claim also reproduces. A Python-version-gated dependency is excluded by `uv tree` for Python 3.13 but remains audited with `uv audit --frozen --python-version 3.13`, including after repinning the interpreter to 3.13. A separate Windows-only dependency fixture likewise remains audited with an explicit Linux target, while `uv tree` excludes it.

No exact duplicate was found in the prior issue/history search. astral-sh/uv#18506 and astral-sh/uv#18185 explain interpreter downloads and the frozen workaround; astral-sh/uv#18511 provides related subset-filtering history.

## Draft response

Reproduced with uv 0.13.5 on Linux x86_64: audit ignores `UV_PYTHON=3.13`, selects the 3.11 project pin, and rejects `--python`. The target flags also fail to filter dependencies: both a Python <3.12 requirement audited for Python 3.13 and a Windows-only requirement audited for Linux remain included.

`uv audit --frozen --preview-features audit-command` avoids interpreter discovery when neither target flag is supplied, but it still performs a universal audit. Interpreter selection and audit-target filtering are separate concerns. Normal locking/download behavior is explained in astral-sh/uv#18506; downloading an interpreter alone does not establish a bug.

## Classification

Observed commands confirm the interpreter-selection omission, rejected --python option, and ineffective target filters. Source inspection additionally shows that audit supplies no explicit interpreter request, exposes no --python/UV_PYTHON binding, and computes but discards the markers from --python-version and --python-platform. The documented filtering behavior is therefore incorrect, independently of whether adding --python is viewed as an enhancement. Historical PRs concern universal defaults and group/extra filtering; no previously fixed target-filtering bug or exact duplicate was established.

An interpreter download during a normal audit is not by itself a bug: audit may update or validate the lockfile. Selecting the interpreter used for locking is also distinct from selecting the target environment whose dependencies should be audited. The established correctness problem is that explicit target flags promise filtering but do not apply it. The missing interpreter override is a separate omission.

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

Searched `crates/uv/tests/it/`, `crates/uv-client/tests/it/`, and the relevant command module, `crates/uv/tests/build/audit.rs`. No direct audit coverage of `UV_PYTHON`, `--python-version`, or `--python-platform` was found. These nearby tests were read through their setup and assertions:

- `audit_no_dependencies` locks an empty project and snapshots a successful audit of zero packages; it has no conflicting interpreter requests.
- `audit_json_no_vulnerabilities` uses a one-package lockfile and mock audit service, then checks frozen JSON output with one audited package. It does not test interpreter discovery or target filtering.
- `audit_dependency_groups` checks all-groups, no-dev, no-group, and only-group package counts. `audit_no_default_groups` checks three packages by default versus one with the flag, despite `default-groups = ["dev"]`. Neither exercises environment markers.

The audit module is gated by both `test-python` and `test-pypi` in `crates/uv/tests/build/main.rs`. No repository tests were added or run; validation used the installed CLI. No checkout files or GitHub state were changed.

## Related

- astral-sh/uv#18506 — Roadmap: `uv audit` (issue, open). Maintainer comments explain that audit normally locks first and recommend --frozen to avoid interpreter downloads. This explains the workaround, but does not track ignored UV_PYTHON or ineffective target filters.
- astral-sh/uv#18185 — Make universal the default for `uv audit` (pull request, merged). Introduced the exact condition that skips interpreter discovery only for frozen audits without --python-version or --python-platform. It establishes the historical behavior, not a fix for interpreter overrides or target filtering.
- astral-sh/uv#18511 — Evaluate extras and groups when determining auditable packages (pull request, merged). Introduced lockfile traversal filtered by extras and dependency groups. Maintainer discussion supports auditing everything by default with explicit subset selection; the implementation does not supply Python/platform markers to that traversal.

## Supporting evidence

Evidence was inspected in checkout commit `564a2074b02eb9de5aaaa9491ed74357ade0332f`.

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

## Maintainer next steps and validation

Wire the explicit interpreter request through audit's CLI, settings, and discovery paths, and pass the target marker environment into auditable dependency selection. Keep the universal default and the meaning of interpreter versus target options distinct.

Add focused regressions for the supplied interpreter-precedence case and target-specific dependency exclusion, including the frozen path. The existing module is `crates/uv/tests/build/audit.rs`; its parent module gates it on both `test-python` and `test-pypi`. Inspection found no explicit `UV_PYTHON`, `--python-version`, or `--python-platform` coverage in that module. Follow its explicit fixtures, mock audit service, and `uv_snapshot!` patterns, and check shared coverage before adding tests.

This handoff combines observed CLI reproductions with source inspection and the previously collected GitHub history. The installed uv 0.13.5 reproduced all core claims, including both target filters; the exact reported uv 0.13.6 Docker environment was not executed. No build or repository test suite was run. The vulnerability counts depend on the advisory service, while the observed dependency-filtering failure does not. Only temporary reproduction artifacts and this issue-context README were written; no checkout files or GitHub state were changed.
