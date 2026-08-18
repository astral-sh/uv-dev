# Provide a way to select the interpreter used for venv creation without overriding installation target of `uv pip`

Issue: astral-sh/uv#21191

Classification: duplicate

## Summary

The report asks for a setting such as `UV_BASE_PYTHON` that selects the base interpreter used by
`uv venv` without also selecting the environment modified by later `uv pip` commands. The motivating
case exports a Nix-managed Python so virtual environments are created from the intended interpreter;
after activating the new environment, the same `UV_PYTHON` value makes `uv pip install` target the
immutable Nix store instead of the activated virtual environment.

astral-sh/uv#14748 is an exact prior request. It proposes `UV_PYTHON_VENV` for the same Nix workflow
and describes the same consequence: `uv venv` uses the desired interpreter, while `uv pip install`
selects the read-only base Python. astral-sh/uv#6645 discusses the broader idea of a lower-precedence
default Python, and astral-sh/uv#7684 shows the same tension between a globally exported
`UV_PYTHON` and `uv pip` environment selection under a different, architecture-specific trigger.
No implementing pull request or requested environment variable was found. The reporter later
closed astral-sh/uv#21191 after identifying `UV_PYTHON_SEARCH_PATH` as a possible existing solution;
they subsequently confirmed that it appears to provide the behavior they wanted. The current source
supports that result, subject to the usage details below. A maintainer has since clarified that the
best overall way for uv to support Nix's model remains undecided.

## Current status and workaround

The reporter closed the issue while initially uncertain whether `UV_PYTHON_SEARCH_PATH` provided
the desired behavior, then confirmed that it appears to work in their environment. They did not
provide the exact value used or a command transcript. Source inspection independently confirms the
relevant command separation:

- `UV_PYTHON_SEARCH_PATH`, added in uv 0.11.8, replaces `PATH` specifically for Python executable
  discovery.
- `uv venv` requests a system interpreter and therefore consults that executable search path.
- `uv pip install` without `--python`, `UV_PYTHON`, `--system`, `--target`, or `--prefix` restricts
  discovery to virtual environments, preferring the environment named by `VIRTUAL_ENV`; system
  executable search paths are excluded in that mode.

Consequently, setting `UV_PYTHON_SEARCH_PATH` to the directory containing the desired Nix
interpreter, leaving `UV_PYTHON` unset, creating the venv, and activating it should make `uv venv`
select the Nix interpreter while allowing `uv pip install` to select the activated venv. Unlike
`UV_PYTHON`, the value is a platform-separated list of directories, not the path to one executable,
and it replaces rather than augments `PATH` for interpreter discovery. Any fallback interpreter
directories must therefore be included explicitly and ordered as desired. This exact Nix sequence
has not been executed as part of the handoff, but the reporter now confirms that the mechanism works
for their use case. The absence of their exact configuration means this is a user-validated
workaround rather than a fully recorded reproduction. The maintainer's follow-up on
astral-sh/uv#21191 also means this should not be treated as an endorsed or final design for Nix
integration.

The previously documented fallback remains valid on older uv versions: scope `UV_PYTHON` to the
creation command, for example `UV_PYTHON=/path/to/python uv venv new-venv`, or pass
`uv venv --python /path/to/python`.

## Classification

This is a duplicate of astral-sh/uv#14748, which tracks the same requested capability, command
interaction, Nix trigger, and read-only installation target. Duplicate takes precedence over the
underlying enhancement classification.

Repository evidence indicates that the current scope of `UV_PYTHON` is intentional rather than a
regression: in astral-sh/uv#6645, a maintainer explains that `UV_PYTHON` is equivalent to
`--python` and has explicit precedence, while astral-sh/uv#7684 recommends avoiding a globally set
`UV_PYTHON` and passing it per command when `uv pip` should discover the activated environment.
The requested venv-only setting would therefore be new configuration behavior. astral-sh/uv#14748
was closed by its author, has no linked closing pull request, and the checkout contains none of the
proposed `UV_BASE_PYTHON`, `UV_PYTHON_VENV`, or `UV_DEFAULT_PYTHON` identifiers.

The later `UV_PYTHON_SEARCH_PATH` finding does not change the duplicate relationship, but it lowers
the need for a new venv-specific variable on uv 0.11.8 and later. It controls discovery rather than
expressing a direct interpreter request, which is why it can affect `uv venv` without overriding
the virtual-only target selection used by a normal `uv pip install` invocation.

## Related

- astral-sh/uv#14748 — Exact prior request for `UV_PYTHON_VENV`. It uses the same Nix scenario and
  explains that global `UV_PYTHON` makes `uv pip install` select a read-only base interpreter. It is
  closed; the reporter closed it without a linked implementation.
- astral-sh/uv#6645 — Open broader request for `UV_DEFAULT_PYTHON`. A maintainer linked it from
  astral-sh/uv#14748. It concerns a lower-precedence default that can yield to project pins, rather
  than specifically limiting interpreter selection to virtual-environment creation.
- astral-sh/uv#7684 — Open adjacent report where exporting `UV_PYTHON` for an architecture-specific
  interpreter prevents `uv pip` from selecting the activated environment. It supports the current
  per-command workaround, but its requested architecture-aware discovery differs from this issue.

## Search and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
included `UV_BASE_PYTHON`, `UV_PYTHON_VENV`, `UV_DEFAULT_PYTHON`, combinations of `UV_PYTHON` with
`uv venv`, `uv pip`, and `VIRTUAL_ENV`, and the Nix external-management, immutable-store, and
read-only symptoms. Conceptual queries covered base/default interpreters, activated-environment
discovery, venv creation, and per-command Python selection. Candidate bodies, comments, closure
metadata, linked issues, and linked pull requests were inspected; no implementing pull request was
found.

The reporter's lead, astral-sh/uv#6612, was inspected but is not the canonical match. It concerns
project commands ignoring active external environments and was resolved by the
`UV_PROJECT_ENVIRONMENT` work in astral-sh/uv#6834. Here, the project interface is not involved:
`uv pip` is obeying an explicitly exported `UV_PYTHON` instead of discovering the activated venv.
astral-sh/uv#12748 was also ruled out because it asks to constrain parent-directory environment
discovery rather than to separate venv base-interpreter selection from the `uv pip` target.

Source inspection for the follow-up checked the environment-variable definition, Python discovery
ordering, and the command-specific environment preferences. It confirms that
`UV_PYTHON_SEARCH_PATH` was added in uv 0.11.8, overrides `PATH` only for executable discovery,
participates in the system-interpreter lookup used by `uv venv`, and is not consulted by the
virtual-only lookup used by an ordinary `uv pip install`.
