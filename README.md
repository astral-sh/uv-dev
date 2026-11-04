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
also recommends explicitly creating the environment with uv sync for editor use. No matching
wording correction or historical fix was found.

## Draft response

“Editor” is intentional here: Python editor integrations use the project environment to resolve imports and provide code completion and type hints. uv sync creates or updates that environment (normally .venv) without running a command, which is useful before you start editing. Configure your editor’s Python integration to use the project’s .venv. This connection is explained at https://docs.astral.sh/uv/concepts/projects/layout/#the-project-environment. The sentence could be clearer as “ensuring the environment your editor uses has the correct versions of dependencies.”

## Classification

The reporter asks why the syncing documentation mentions an editor and suggests substituting project. The project-layout documentation explicitly explains that editors use the project environment for code completion and type hints, and the migration guide independently recommends uv sync for editor use. The wording describes intended behavior; no incorrect behavior, regression, or established documentation error is reported. The related discussions do not track this specific clarification.

The issue currently carries the bug label, but the report and documentation support classifying it
as a question. Clarifying the sentence or linking to the project-environment explanation would
address the ambiguity without changing the documented behavior. No reproduction or version
information is needed to answer the question.

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

Answer the environment/editor connection directly using the draft above. A small wording
clarification or cross-reference is a reasonable documentation follow-up, but no implementation
change is established by this report. This handoff and draft are for maintainer review; no GitHub
changes were made.
