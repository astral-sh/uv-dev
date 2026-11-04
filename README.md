# Documentation: uv sync, editor-project?

Issue: astral-sh/uv#23102
Classification: question

## Summary

The reporter questions the sentence “Syncing the environment manually is especially useful for
ensuring your editor has the correct versions of dependencies” in the syncing concepts page. They
suggest replacing “editor” with “project” because they do not see a connection between the project
environment and their editor. They report no command failure or editor malfunction; platform and
uv version are both “Any.”

The reference to an editor is intentional. The project-layout documentation explicitly explains
that editors use the project environment for code completion and type hints. The migration guide
also recommends explicitly creating the environment with uv sync for editor use. A temporary
project experiment confirmed that uv sync creates .venv and installs a pinned dependency that is
then importable through .venv/bin/python without invoking uv run. This establishes the environment
behavior, not any particular editor’s discovery or completion behavior. No matching wording
correction or historical fix was found.

Maintainer zanieb has now confirmed the intended distinction in astral-sh/uv#23102
(comment 6635554960): uv run syncs automatically, so a separate uv sync is unnecessary for
that workflow. Explicit uv sync populates the virtual environment with packages needed by
language-server (LSP) integrations in an editor. This directly answers the conceptual question;
it does not report a defect or establish a documentation-change decision.

The reporter accepts the LSP explanation and clarifies the remaining documentation request in
astral-sh/uv#23102 (comment 6637534303): explicitly introduce LSP integrations so the editor
reference has context, and mention exact syncing as another benefit of uv sync. The latter is
supported by the same page: uv sync removes extraneous packages by default, whereas uv run retains
them unless --exact is used. These are concrete documentation suggestions, not a new failure
report or a maintainer commitment to change the page.

## Draft response

This earlier draft is retained as background; the maintainer has since supplied the explanation.

“Editor” is intentional here: Python editor integrations configured to use the project environment can inspect its installed dependencies to resolve imports and provide code completion and type hints. uv sync creates or updates that environment (normally .venv) without running a command, which is useful before you start editing. Configure your editor’s Python integration to use the project’s .venv. This connection is explained at https://docs.astral.sh/uv/concepts/projects/layout/#the-project-environment. The sentence could be clearer as “ensuring the environment your editor uses has the correct versions of dependencies.”

## Classification

The reporter asks why the syncing documentation mentions an editor and suggests substituting project. The project-layout documentation explicitly explains that editors use the project environment for code completion and type hints, and the migration guide independently recommends uv sync for editor use. The wording describes intended behavior; no incorrect behavior, regression, or established documentation error is reported. The related discussions do not track this specific clarification.

The maintainer clarification also supports the question classification: the editor reference
describes the intended use of manual synchronization, rather than an incorrect term.

The issue currently carries the bug label, but the report and documentation support classifying it
as a question. Clarifying the sentence or linking to the project-environment explanation would
address the ambiguity without changing the documented behavior. The conceptual question can be
answered from the documentation and the environment demonstration below. There is no reported
command or editor failure to classify as reproduced or not reproduced.

The original question has been answered and accepted by the reporter. The follow-up asks for a
documentation improvement rather than alleging incorrect behavior; retain the original question
classification while recording that narrower documentation follow-up for maintainer review.

## Reproduction

Outcome: **needs_more_information**. The report asks whether “editor” should be replaced with
“project”; it supplies no failing command, project configuration, expected-versus-actual editor
behavior, or named editor. Platform and uv version are “Any,” and no Python version is supplied.
No behavioral defect is alleged, so a defect reproduction does not apply. The experiment below
helps answer the conceptual question; it is not a reproduction of an editor failure.

### Observed environment behavior

Used the installed uv executable from PATH:
`/opt/hostedtoolcache/uv/0.13.6/x86_64/uv`, reporting
`uv 0.13.6 (x86_64-unknown-linux-gnu)`. Host: Linux 7.0.0-1014-aws, x86_64; interpreter:
CPython 3.12.3, selected explicitly from the existing Python installation. No uv build or repository
test execution was needed.

Created `/tmp/uv-23102-oabu0_tn/project/pyproject.toml` with:

```toml
[project]
name = "editor-environment-example"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["idna==3.10"]
```

There was initially no `.venv` or lockfile. There are no workspaces, dependency groups, optional
dependencies, uv sources, or build system in this example. All fixture files, uv caches, and
HOME/XDG/Python-install/temp directories were placed under `/tmp/uv-23102-oabu0_tn`. Commands ran
with a fresh environment, automatic Python downloads disabled, and configuration discovery
disabled. No existing project or user environment was changed.

From the fixture directory, the relevant commands were (with `<python>` denoting the explicitly
selected existing CPython 3.12.3 executable):

```console
uv sync --python <python> --no-python-downloads --no-config --no-build --default-index https://pypi.org/simple
.venv/bin/python -c 'import idna; from importlib.metadata import version; print("idna version:", version("idna")); print("idna location:", idna.__file__)'
```

Both commands exited 0. Sync resolved two packages, created `uv.lock` and `.venv`, and installed
`idna==3.10`. The direct interpreter probe printed:

```text
idna version: 3.10
idna location: /tmp/uv-23102-oabu0_tn/project/.venv/lib/python3.12/site-packages/idna/__init__.py
```

No application command or `uv run` was required to prepare the environment. An editor integration
configured to use that environment can inspect these installed dependencies. No editor was
launched, and this experiment does not establish automatic editor discovery, language-server
configuration, or completion behavior. The documentation explicitly describes that intended
connection in `docs/concepts/projects/layout.md` under “The project environment.”

### Existing test coverage and limits

Read the relevant integration-test setup and assertions:

- `crates/uv/tests/sync/sync.rs`, `sync`: creates a Python >=3.12 project depending on iniconfig;
  snapshots successful resolution and installation of iniconfig 2.0.0 and asserts that `uv.lock`
  exists. This covers basic manual synchronization, not editor integration.
- `crates/uv/tests/sync/sync.rs`, `check`: uses the same dependency fixture; snapshots an outdated
  environment from `uv sync --check`, successful installation by `uv sync`, and then a successful
  check reporting an up-to-date lockfile and no changes. This covers synchronization state, not
  editor behavior.

These modules are gated on `test-python` and `test-pypi` in `crates/uv/tests/sync/main.rs`.
No test was added or run, and no coverage claim is made for an editor failure.

To investigate an actual malfunction, obtain the editor and Python extension/language-server
versions, selected interpreter/environment path, operating system, uv and Python versions,
`pyproject.toml` (including any workspace/group/source configuration), exact commands and output,
and a concrete example of expected versus observed import/completion/type-hint behavior. Those
inputs are unnecessary for answering the wording question itself.

## Related

- astral-sh/uv#10813 — How can I install project dependencies from an existing pyproject.toml? (closed).
  Related documentation clarification: a later comment quotes the exact same syncing paragraph. A maintainer explains that uv sync installs project dependencies into .venv and uv run synchronizes automatically. The discussion concerns discovering and understanding dependency installation, rather than whether editors use the environment, so it is not a duplicate.
  The maintainer explanation is in comment 2604830673; comment 3732230307 quotes the disputed
  paragraph while asking what syncing does.
- astral-sh/uv#9637 — Selecting .venv/bin/python as an active interpreter in VSCode doesn't enable IDE features (closed).
  Adjacent editor-integration evidence: this discussion connects the selected .venv interpreter to import resolution, completion, and type hints. Maintainers describe uv environments as standards-compliant and direct the actual selection failure to VS Code. This report asks about documentation wording and does not report that integration failure.
  Maintainer comment 2628320939 explains that standard virtual environments named .venv should
  be discoverable by the editor. The closing comment 3943379021 reports that the proposed editor
  extension solution appears to work; this does not establish a uv sync defect.

## Supporting evidence

- astral-sh/uv#23102, reporter comment 6637534303 (2026-11-04), accepts the explanation and
  asks for explicit LSP context plus mention of exact syncing. This is the reporter's proposal;
  the supplied discussion contains no maintainer acceptance of these documentation changes.
- docs/concepts/projects/sync.md:92–108 documents the exact-sync distinction: uv sync removes
  packages not selected for installation by default; --inexact retains them. uv run retains
  extraneous packages by default and supports --exact to remove them. Thus manual sync is not
  the only way to request exact syncing, and preparing an editor environment is not its only use.
- astral-sh/uv#23102, comment 6635554960 by maintainer zanieb (2026-11-04), explicitly
  identifies editor LSP integrations as the reason to populate the environment with uv sync
  when automatic synchronization through uv run is not being invoked. The supplied full issue
  discussion identifies the commenter as a repository member.
- docs/concepts/projects/sync.md:3–5 defines syncing as installing a subset of locked packages into
  the project environment. Lines 9–10 explain that uv run automatically locks and syncs before
  running a command. Lines 67–77 contain the manual uv sync command and disputed editor sentence.
- docs/concepts/projects/layout.md:29–44 explains the persistent project environment, normally
  .venv next to pyproject.toml. Lines 34–35 explicitly say editors need the environment for code
  completions and type hints, and lines 42–44 distinguish automatic creation with uv run from
  explicit creation with uv sync.
- docs/guides/migration/pip-to-project.md:479–485 explains automatic checks before uv run and says
  “The project environment can also be explicitly created with uv sync, e.g., for use with editors.”
- Public explanation for the reporter:
  https://docs.astral.sh/uv/concepts/projects/layout/#the-project-environment.
- astral-sh/uv#10813 confirms the installation workflow in a maintainer reply. The later request
  quoting the syncing paragraph links onward to astral-sh/uv#5605, where it asks for an explanation
  of the steps performed by uv sync. Neither discussion specifically challenges the relationship
  between an editor and its configured Python environment.
- astral-sh/uv#8040 requests automatically generated Pyright environment configuration. Its
  maintainer response declines configuration for external tools. It concerns configuration policy,
  rather than the correctness of the syncing sentence, and is not included as a closest match.
- astral-sh/uv#5605 is a general documentation-feedback tracker. Its relevant comments concern
  sync discoverability, installation steps, and defaults. It does not establish a canonical
  discussion of this editor-versus-project question.

## Search scope

The report was decomposed into two connected claims before searching:

1. A proposed wording correction from “editor” to “project” in the uv sync documentation.
2. A request to explain how installed project dependencies relate to an editor.

The affected subsystem is project-environment synchronization and its documentation. Exact
identifiers include uv sync, the syncing-the-environment anchor, and the quoted manual-sync
sentence. There is no error message, specific editor, version-dependent trigger, or regression
claim. Observable wording and editor-use questions were searched separately from potential
editor-discovery or language-server configuration failures.

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests using authenticated gh. Literal searches covered the quoted sentence, correct versions of dependencies, syncing-the-environment, and editor/sync/documentation wording. Conceptual searches covered IDEs, project environments, virtual environments, code completions, and language servers; fix-oriented PR searches covered editor integration, sync documentation, and clarify sync. Inspected astral-sh/uv#10813 and its comments, followed its documentation-feedback connection to astral-sh/uv#5605, and inspected astral-sh/uv#9637 and astral-sh/uv#8040. Ruled out astral-sh/uv#5605 as a duplicate because it is a broad feedback tracker whose linked sync request concerns explaining installation steps; astral-sh/uv#8040 requests generated Pyright configuration. No matching pull request or prior fix for the reported wording was found.

## Handoff

The maintainer has answered the environment/editor connection, and the reporter accepts that
explanation. The remaining request is to improve the documentation by explicitly introducing LSP
integrations and mentioning exact syncing as another use of uv sync. Review whether to add that
context or a cross-reference to “Handling of extraneous packages.” Any clarification should explain
that exactness is a default difference, configurable with uv sync --inexact and uv run --exact;
it should not imply that exact syncing requires uv sync.

No maintainer commitment to these edits appears in the supplied discussion. No additional public
response is prepared. The original classification remains question, with the documentation
suggestions recorded above. The structured reproduction outcome remains needs_more_information
because no behavioral failure is specified; the successful synchronization experiment supports the
explanation without claiming to test an editor. The follow-up supplies no failing reproduction,
affected version or platform, or editor configuration. No GitHub changes or checkout modifications
were made.
