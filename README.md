# Support --relocatable and UV_VENV_RELOCATABLE with uv sync

Issue: astral-sh/uv#21945

Classification: duplicate

## Summary

The issue requests two equivalent opt-in interfaces for project environments:
`uv sync --relocatable` and `UV_VENV_RELOCATABLE`. When no project environment exists, either interface
should make the implicitly created `.venv` use the same relocatable semantics as
`uv venv --relocatable`. The report also asks for a design decision when `.venv` already exists, suggesting
either a warning or rewriting installed scripts.

astral-sh/uv#5737 is the canonical open match. It already requests that `uv sync` accept
`--relocatable` when it implicitly creates `.venv`, along with persistent configuration for that
virtual-environment option. The new report's environment-variable spelling and its explicit
existing-environment case add useful detail, but do not require a separate discussion.

There is also an experimental alternative. Merged astral-sh/uv#19965 makes project environments
relocatable under `--preview-features relocatable-envs-default`. This is a default-behavior preview,
not the direct `uv sync --relocatable` or `UV_VENV_RELOCATABLE` interface requested here.

The follow-up comment identifies an additional limitation: after `uv venv --relocatable && uv sync`,
the default editable installation of the current workspace can retain an absolute project path, so
moving the project and its `.venv` together is not sufficient to relocate the workspace import.
That behavior is already tracked in astral-sh/uv#18208. The commenter suggests either rewriting the
editable path relative to the environment or coupling relocatable syncs with `--no-editable`; the
latter is a proposed design/workaround and has not been established here as the required behavior.

## Draft response

Thanks for the request. This is already tracked in astral-sh/uv#5737, which explicitly asks for
`uv sync` to accept relocatable virtual-environment creation options when it creates `.venv`; the
behavior for an existing environment can be worked out in that discussion as well.

As a current experimental alternative, `--preview-features relocatable-envs-default` makes project
environments created by `uv sync` relocatable, via astral-sh/uv#19965. That does not add the
requested `uv sync --relocatable` or `UV_VENV_RELOCATABLE` interface. Let's centralize the opt-in
interface request in astral-sh/uv#5737.

## Classification

Duplicate of astral-sh/uv#5737. That open issue names the same command, implicit `.venv` creation,
and `--relocatable` option, and proposes allowing the option to be passed to `uv sync`. It is broad
enough to centralize the environment-variable form and the existing-environment semantics raised by
astral-sh/uv#21945.

This is not a bug or regression: the report asks uv to expose additional functionality on `uv sync`
and does not establish that the documented current behavior is incorrect. Ordinarily that would be
an enhancement, but the direct open match makes `duplicate` take precedence.

## Related

- astral-sh/uv#5737 — Exact canonical match. It requests that `uv sync` accept `--relocatable`
  when implicitly creating `.venv`, as well as persistent configuration for virtual-environment
  options.
- astral-sh/uv#13994 — Broader alternative. It proposes relocatable project environments by default
  rather than the explicit flag/environment-variable opt-in requested here, and it links back to
  astral-sh/uv#5737.
- astral-sh/uv#19965 — Merged pull request implementing the broader alternative for project
  environments behind `--preview-features relocatable-envs-default`; it does not add
  `uv sync --relocatable` or make `UV_VENV_RELOCATABLE` a sync setting.
- astral-sh/uv#10325 — Adjacent existing-environment design. It concerns when `uv sync` recreates an
  environment and specifically notes that creation flags such as `--relocatable` may be lost, but
  requests a fail-instead-of-recreate control rather than relocatable sync creation.
- astral-sh/uv#18208 — Exact match for the follow-up's editable-workspace limitation. It reports that
  syncing a workspace into a relocatable environment leaves an absolute path in the editable
  project's `.pth` file. A maintainer notes there that the `.pth` file comes from the build backend
  and suggests that uv may need to rewrite paths for relocatable environments.
- astral-sh/uv#18331 — Merged pull request that added `UV_VENV_RELOCATABLE` for `uv venv`. It
  establishes the exact environment variable's current command scope but does not provide the
  requested `uv sync` interface.

## Supporting evidence

Current source passes the `relocatable-envs-default` preview state into project-environment creation,
and the sync integration test verifies that the resulting `pyvenv.cfg` contains
`relocatable = true` and that a console script still runs after the project is moved. The preview was
expanded from `uv venv` to project environments by astral-sh/uv#19965. By contrast, the direct
`--relocatable` argument and `UV_VENV_RELOCATABLE` setting are defined for `uv venv`, consistent with
the remaining interface request in astral-sh/uv#5737.

The existing sync preview test installs `black`, moves the project directory, and verifies that the
`black` console script still runs. It does not install the current workspace as an editable package
or verify imports from that workspace after the move, so it does not cover the absolute editable
path reported in the follow-up. astral-sh/uv#18208 provides the closer evidence for that case; its
maintainer discussion attributes the `.pth` content to the wheel produced by the build backend and
leaves path rewriting as a possible uv-side solution rather than a confirmed design.

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
used `--relocatable`, `UV_VENV_RELOCATABLE`, `uv sync --relocatable`, and the `uv sync`/`uv venv`
pairing. Conceptual queries covered implicit project-environment creation, virtual-environment
options and configuration, existing-environment recreation, moved environments, entrypoint/script
rewriting, and relocatable defaults. Candidate bodies, comments, closing pull requests, linked
discussions, and current source/tests were inspected.

astral-sh/uv#10895 and astral-sh/uv#13989 were plausible moved-environment candidates but were ruled
out as direct matches: they concern detecting or reconstructing already-broken environments after a
move, not choosing relocatable semantics when `uv sync` creates an environment. Relocatable export
and distribution requests such as astral-sh/uv#6970 and astral-sh/uv#2389 were also ruled out because
they concern packaging or distributing whole environments rather than the `uv sync` creation
interface.
