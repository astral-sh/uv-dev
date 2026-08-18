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
No implementing pull request or requested environment variable was found.

## Draft response

Thanks — this is the same request as astral-sh/uv#14748. That report likewise asks for a
venv-scoped Python setting because exporting `UV_PYTHON` selects the desired Nix interpreter for
`uv venv` but also directs `uv pip` to the read-only base interpreter instead of the activated
environment.

`UV_PYTHON` currently behaves like `--python` and is not limited to venv creation. For now, the
workaround is to scope it to the creation command, for example
`UV_PYTHON=/path/to/python uv venv new-venv`, or pass
`uv venv --python /path/to/python`. The earlier issue was closed by its author without a linked
implementation; the broader default-interpreter discussion remains open in astral-sh/uv#6645. We
should centralize this venv-scoped request in astral-sh/uv#14748.

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
