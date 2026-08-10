# centralized-project-envs: disable creation of .venv symlink

Issue: astral-sh/uv#21034

Classification: enhancement

## Summary

The report enables the `centralized-project-envs` preview feature and asks for a way to keep the
project directory completely untouched instead of publishing the cached environment through a
project-local `.venv` symlink. It also reports that a pathless `uv venv` invocation fails when the
project directory is read-only because the link cannot be created.

The opt-out is not currently available. The `.venv` reference is an intentional compatibility
mechanism for activation, editors, and other tools: astral-sh/uv#18214 introduced centralized
project environments with a symlink or junction, and astral-sh/uv#19912 extended that behavior to
pathless `uv venv` invocations from a project root. astral-sh/uv#20022 later added a plain `.venv`
path file as the fallback when a link cannot be created.

The reported hard failure is not established by the information in the issue and conflicts with
the current implementation and documentation. Current source treats link publication as
best-effort: it tries a link, then a path file, and continues with the cached environment if both
fail. The `create_centralized_project_environment_link_failure` integration test also expects
`uv venv` to succeed when `.venv` cannot be replaced. The reporter's uv version, operating system,
filesystem, exact command and complete error output are needed to determine whether this is a
current bug, an older-version behavior, or a failure elsewhere that was attributed to link
creation.

## Draft response

The `.venv` link is currently intentional: centralized environments use it as a compatibility
pointer so activation, editors, and other tools can still discover the project environment. When a
link cannot be created, current uv should fall back to a `.venv` path file; if that also cannot be
written, uv is intended to continue using the cached environment directly. This behavior was added
across astral-sh/uv#18214, astral-sh/uv#19912, and astral-sh/uv#20022.

An option to omit the project-local reference entirely would be a new capability, and this issue
can track that request. It would mean tools that rely on `.venv` may not discover the environment.

The `uv venv` failure on a read-only project is not expected from the current implementation. Could
you provide the uv version, operating system and filesystem, exact command, complete output, and a
minimal reproduction? That will show whether the non-fatal fallback has a bug or the failure occurs
at a different step.

## Classification

This is an enhancement because the primary request is a new option that suppresses all `.venv`
publication while centralized storage is enabled. Existing behavior deliberately creates a
compatibility reference, and no setting or flag currently disables it.

The read-only failure would be a bug if reproduced against a current release, because both the
design in astral-sh/uv#18214 and current source say link creation failure is non-fatal. The report
does not include a version, error, platform, or reproduction sufficient to establish that failure,
while current integration coverage explicitly expects success after link publication fails.
Therefore the unconfirmed failure does not override the concrete capability request for this
triage.

This is not a duplicate of astral-sh/uv#1495. That issue originated centralized environment
storage and includes discussion that some users do not want even a symlink in the project, but it
was closed when astral-sh/uv#18214 implemented centralized storage with the link as a deliberate
part of the design. It does not separately track an opt-out from that design.

## Related issues and pull requests

- astral-sh/uv#18214 (merged), “Centralised environment storage” — the closest design and
  implementation reference. It introduced the preview feature, deliberately maintained `.venv` as
  a compatibility pointer, and specified that inability to create the link must be non-fatal.
- astral-sh/uv#19912 (merged), “Support centralised environments in `uv venv`” — the closest result
  for the reported command. It extended centralized storage to pathless `uv venv` and added an
  integration test in which link publication fails but the command succeeds.
- astral-sh/uv#20022 (merged), “Support `.venv` as a file containing a path fallback mechanism for
  centralised project environments” — added the fallback used when symlinks or Windows junctions
  cannot be created. It still writes `.venv`, so it does not satisfy the requested no-reference
  mode.
- astral-sh/uv#1495 (closed), “Add an option to store virtual environments in a centralized
  location outside projects” — the originating feature request. Its discussion anticipated the
  IDE-discovery reason for a symlink and also records use cases where a symlink remains unwanted;
  it was closed by astral-sh/uv#18214 rather than retaining the narrower opt-out request.

## Search scope and exclusions

Authenticated GitHub searches covered open and closed issues and open, closed, and merged pull
requests. Literal searches included `centralized-project-envs`, `centralized project envs`, `.venv
symlink`, `venv symlink disable`, `venv symlink read-only`, `read-only uv venv`, and the warning
fragments `Failed to create link to project environment` and `Failed to write the environment
path`. Conceptual searches covered centralized or external project environments, project
environment links, suppressing IDE compatibility pointers, and read-only project roots.
Fix-oriented review followed the implementation history from astral-sh/uv#1495 through
astral-sh/uv#18214, astral-sh/uv#19912, and astral-sh/uv#20022, then compared current documentation,
source, tests, comments, and merge timing.

The following plausible results were inspected but excluded from the related set:

- astral-sh/uv#20247 requests named environments shared independently of projects; it explicitly
  distinguishes that request from centralized per-project environments and does not concern
  suppressing `.venv`.
- astral-sh/uv#20060 asks for a CLI equivalent of `UV_PROJECT_ENVIRONMENT`; it changes explicit
  environment selection rather than the centralized feature's compatibility reference.
- astral-sh/uv#7642 concerns centralized placement and HPC filesystem constraints, but its nearest
  discussion uses `.venv` symlinks as a workaround and does not track a link opt-out.
- astral-sh/uv#20433 intentionally errors on discovery of an already broken `.venv` symlink to
  prevent selecting an unrelated ancestor environment. That is different from failure to publish a
  new compatibility link for a valid cached environment.
