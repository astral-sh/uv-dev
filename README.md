# Support limiting Python versions for building

Issue: astral-sh/uv#20759

Classification: duplicate

## Summary

The reporter runs `uv sync` and `uv run` for a project with a broad `requires-python` range in an
air-gapped environment. Project-mode universal resolution reaches a
`python_full_version >= '3.15'` split and fails because the available mirror contents cannot satisfy
that split, even though the reporter only needs the installed Python 3.12.3 environment. They ask
for a local override that narrows resolution without changing the project's published
`requires-python` metadata: either make `.python-version` constrain resolution or allow a Python
range in global configuration.

This local-resolution capability is already tracked by astral-sh/uv#7889. That issue has the same
universal-versus-selected-Python behavior, the same need to avoid temporarily rewriting
`requires-python`, and explicitly proposes a local resolution mode for `uv sync` and `uv run`.
astral-sh/uv#16070 is a second close report involving `uv run --python 3.13`; it confirms that
selecting an interpreter does not currently disable universal project resolution.

Repository documentation and tests establish that `.python-version` chooses the interpreter for
the project environment, while `[tool.uv].environments` limits universal resolution and accepts
Python markers. However, `environments` is rejected in `uv.toml`, including user-level/global
configuration, so the existing setting does not satisfy the requested untracked or global
override.

## Draft response

Thanks for clarifying that this occurs during `uv sync` and `uv run`. Project commands currently
perform a universal resolution over the Python versions declared by `requires-python`;
`.python-version` selects the interpreter but does not narrow that resolution.

The request for a local resolution mode based on the selected Python and platform, without changing
the project's `requires-python`, is already tracked in astral-sh/uv#7889. A closely related example
for `uv run --python` is in astral-sh/uv#16070, so let's centralize the feature discussion in
astral-sh/uv#7889.

Today, `[tool.uv].environments` can narrow resolution with a Python marker, but that setting is
project configuration and is not accepted in a global `uv.toml`. That means it does not provide the
local/global override requested here.

## Classification

This is a duplicate of astral-sh/uv#7889. Both issues request non-universal, current-environment
resolution for project commands because a broad project `requires-python` range can make unrelated
Python branches fail, and both specifically want to avoid editing `pyproject.toml` to test or use one
Python version. The new report adds an air-gapped mirror and proposes `.python-version` or global
configuration as interfaces, but those are additional motivations and interface suggestions for
the same underlying capability.

The observed attempt to resolve other declared Python versions is the documented project-mode
universal-resolution behavior, so the report does not establish a correctness regression. Without
the existing canonical request it would be an enhancement; the duplicate classification takes
precedence.

## Related

- astral-sh/uv#7889 — Open issue requesting `--resolution=local` or equivalent for `uv sync` and
  `uv run`, limited to the selected Python/platform, specifically to avoid rewriting a broad
  `requires-python` range. This is the canonical duplicate.
- astral-sh/uv#16070 — Open issue where `uv run --python 3.13` still performs universal resolution
  and fails on a Python 3.8 branch. It is the same behavior and requested escape hatch, but its
  trigger is a temporary nightly package index rather than an air-gapped mirror.
- astral-sh/uv#18745 — Open question about the upper Python range of universal resolution. A
  maintainer explains that uv does not compute a general maximum Python version; this is adjacent
  context for the reported 3.15 split, not a tracker for local project resolution.

## Search evidence

Searches covered the literal resolver hint, `python_full_version >= '3.15'`, `keyring`, and
`pywin32-ctypes`; conceptual terms including local or non-universal resolution, selected/current
Python, `.python-version`, `requires-python`, supported environments, global `uv.toml`, air-gapped
and offline operation; and closed issues and merged pull requests involving Python 3.15, custom
indexes, forking, and late `Requires-Python` metadata. Candidates and their maintainer comments were
compared across commands, resolution mode, requested configuration scope, trigger, and confirmed
mechanism.

astral-sh/uv#19911 and its merged fix astral-sh/uv#20586 were inspected because they involve a
custom index, universal forking, and Python 3.15. They address a different bug: failure to fork when
`Requires-Python` is learned late from wheel metadata. Although the reporter's uv 0.11.28 predates
that merged fix, the present error is an unsatisfied dependency branch rather than failure to retain
a previously selected candidate, so they were ruled out. No pull request implementing local
project resolution was found.
