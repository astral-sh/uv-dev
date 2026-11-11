# Audit installed Python version(s) for vulnerabilities

Issue: astral-sh/uv#23224
URL: https://github.com/astral-sh/uv/issues/23224

Classification: enhancement

## Summary

The reporter requests an explicit way to check uv-managed Python installations for known vulnerabilities, motivated by the eMudhra CA compromise. Suggested interfaces include `uv python audit`, selecting the current project interpreter by default, `--all` for installed versions, and `--default` for the interpreter exposed by `uv python install --default`. These are design suggestions, not established command behavior.

A separate alternative is to include the project's Python version from `.python-version` in `uv audit`. The reporter raises a prospective compatibility concern for JSON consumers that currently expect package findings; no broken workflow is demonstrated.

No duplicate found. astral-sh/uv#8217 covers Python patch-update notifications; astral-sh/uv#19886 and merged astral-sh/uv#20921 provide the installed-tool auditing precedent; astral-sh/uv#18506 tracks the existing package-audit roadmap.

## Draft response

Current `uv audit` and `uv tool audit` check locked package dependencies; they do not audit the Python interpreter itself. Your proposal would add a new capability. astral-sh/uv#8217 covers patch-update notifications, while astral-sh/uv#20921 provides an interface precedent for auditing installed tools.

Before settling on a command or extending the package-oriented JSON output, we'd need to define the advisory source and whether coverage includes CPython, bundled libraries, and certificate stores. Could you share an example advisory and an installed Python build you would expect it to flag? That would give this proposal a concrete starting point for design.

## Classification

The report requests new vulnerability-auditing functionality for installed Python interpreters and proposes command selection and reporting designs. Source confirms that current audit operations query locked package dependencies in the PyPI ecosystem and that PythonCommand has no audit subcommand. No malfunction or regression is established; the JSON compatibility concern is prospective. The closest discussions cover patch notifications or package auditing, so they do not centralize this request.

The request is distinct from notification that a newer patch release exists: an advisory audit would identify known affected installations. Existing tool auditing is an interface precedent, but its audit objects are locked package dependencies. The report does not establish that any particular managed Python build is affected by the motivating CA incident.

## Related

- astral-sh/uv#8217 — Notify users when a new Python patch version is available (issue, open). Shares the goal of keeping Python installations secure: its example requests notification when a CPython patch with security fixes is released. It tracks patch-update notifications, not advisory-based auditing of current, default, or all installed interpreters; maintainer comments discuss updating existing virtual environments.
- astral-sh/uv#19886 — Support `uv tool audit` (issue, closed). Tracks the installed-tool auditing workflow explicitly cited as the proposed interface model. It audits tools' locked package resolutions, not Python interpreter builds, and was implemented by astral-sh/uv#20921.
- astral-sh/uv#20921 — Implement `uv tool audit` (pull request, merged). Merged on 2026-08-04 and closed astral-sh/uv#19886. Provides the closest implemented precedent for selecting installed objects and rendering JSON/SARIF reports. Maintainer review explicitly requests tool names or --all for consistency with uv tool upgrade; it adds package auditing for tools, not interpreter vulnerability checks.
- astral-sh/uv#18506 — Roadmap: `uv audit` (issue, open). Canonical roadmap for existing dependency auditing, advisory services, and machine-readable output. Relevant to the proposal to include the project's Python version in uv audit, but its listed work does not track interpreter auditing. A maintainer explains that interpreter discovery/downloads support locking, rather than auditing Python itself.

## Supporting evidence

- `crates/uv-cli/src/lib.rs:5805` defines `PythonCommand`. Its subcommands manage discovery, installation, upgrades, pinning, removal, and shell paths; there is no interpreter audit command.
- `crates/uv-cli/src/lib.rs:1038` describes `uv audit` as auditing project dependencies. `crates/uv-cli/src/lib.rs:5139` describes `--python-version` as choosing the target Python version for dependency auditing, not requesting vulnerability checks on that interpreter.
- `crates/uv-audit-operations/src/lib.rs:69` builds vulnerability queries from the auditable lockfile's package names and versions. `crates/uv-audit/src/service/osv.rs:335` sends these queries with ecosystem `PyPI`.
- `crates/uv-tool-commands/src/audit.rs:204` invokes the shared lockfile audit for tools. The merged implementation and maintainer review in astral-sh/uv#20921 establish installed-tool selection as relevant prior art; the PR's initial body and the final reviewed selection interface differ, so the review is the stronger evidence for requiring named tools or `--all`.
- `crates/uv-audit-operations/src/json.rs:16` models package audit reports with summary, vulnerabilities, and adverse statuses. This supports considering output design explicitly, but does not prove that adding an interpreter report would necessarily break JSON consumers.
- In astral-sh/uv#18506, a maintainer explains that Python discovery/downloads occur to support the locking phase; `--frozen` can avoid that phase. This does not imply that Python itself is currently audited.
- astral-sh/uv#8217 asks to notify users about newer CPython patches with security fixes. Its linked discussion in astral-sh/uv#1795 concerns keeping virtual environments on updated interpreters, rather than checking vulnerability advisories.

## Candidates ruled out

astral-sh/uv#23034 is a plausible terminology match because it discusses `uv audit`, `.python-version`, and Python selection. Its report concerns ignored `UV_PYTHON`, unwanted interpreter downloads, and dependency marker filtering. It does not request or implement interpreter vulnerability auditing.

astral-sh/uv#22789 shares the eMudhra motivation, but concerns system `ca-certificates` in derived Docker images. The reporter confirmed uv 0.13.5 fixed that image issue. It is not evidence of a managed-interpreter audit feature, or a regression of one.

astral-sh/uv#16646 was closed as a duplicate of astral-sh/uv#9189, the historical request for auditing project dependencies. Following that chain led to astral-sh/uv#18506 and merged scaffolding astral-sh/uv#18119. Merged astral-sh/uv#18124 adds package-audit service types, and merged astral-sh/uv#18937 adds tool lockfiles. These establish package-audit history and prerequisites, rather than a prior implementation of this interpreter-auditing request.

## Search scope

Searched astral-sh/uv with authenticated gh across open and closed issues and open, closed, and merged pull requests. Decomposed interpreter auditing and current/default/all selection separately from including .python-version findings in uv audit and its JSON output. Literal searches included "uv python audit", "uv tool audit", ".python-version" with audit, eMudhra, and CPython/CVE; conceptual searches covered interpreter/runtime auditing, Python vulnerabilities/security/advisories/scanning, python-build-standalone, and outdated patch versions. Broadened to audit, security, vulnerability, vulnerabilities, and CVE, and used the area:uv-audit context learned from candidates. Inspected candidate bodies/comments, followed astral-sh/uv#8217 to astral-sh/uv#1795, astral-sh/uv#16646 to astral-sh/uv#9189 and the roadmap, and astral-sh/uv#19886 to merged astral-sh/uv#18937 and astral-sh/uv#20921. Inspected historical merged audit work astral-sh/uv#18119 and astral-sh/uv#18124; no interpreter-audit fix or regression was established. Ruled out astral-sh/uv#23034, which concerns interpreter selection and dependency filtering during package audits, and astral-sh/uv#22789, whose eMudhra-related Docker system certificate issue was reported fixed in uv 0.13.5. No existing discussion tracking the same interpreter-auditing capability was found.

## Design follow-up

The next useful input is one concrete advisory paired with the installed Python build expected to be flagged. Establish the advisory source and desired coverage for the interpreter, bundled libraries, and certificate stores before deciding command defaults, current/default/all selection, or a JSON representation. No implementation schedule or interface commitment is established by the reviewed discussions.

## Handoff status

Only this temporary issue-context README was updated. No checkout files were modified and no GitHub changes were made. This is a proposed classification and reply for maintainer review.
