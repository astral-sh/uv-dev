# Let a project advertise its run target to uvx (project-side equivalent of pipx's [pipx.run])

Issue: astral-sh/uv#21135

Classification: duplicate

## Summary

The issue asks for project metadata that tells `uvx` / `uv tool run` what to execute when a
distribution is intended to run through `python -m <module>` or a `module:callable` rather than a
matching console script. Suggested designs include honoring `[pipx.run]`, adding uv-specific
metadata, or falling back to a package's `__main__.py`.

The underlying project-side custom-entrypoint request is already tracked by astral-sh/uv#3878.
That issue uses `build` as the example: pipx can run the package through its special entry point,
while `uv tool run` does not know how to select the package's module runner. A maintainer explicitly
identified the package's pipx-specific entry point as the relevant mechanism. The new report
generalizes the same request and adds current pipx uv-backend motivation, but it does not require a
separate canonical discussion.

## Draft response

Thanks for the detailed proposal. The project-side capability is already tracked in
astral-sh/uv#3878: that issue covers packages intended to run via `python -m` and asks `uv tool run`
to support the custom entry-point mechanism used by pipx. The caller-side `-m` shorthand is tracked
separately in astral-sh/uv#7552.

Let's centralize the project-advertised default runner and metadata design in astral-sh/uv#3878, so
I'm marking this as a duplicate. The current explicit form remains `uvx --from <package> python -m
<module> ...`.

## Classification

This is a duplicate of the open astral-sh/uv#3878, which tracks the same underlying capability:
selecting a custom package runner when the package does not expose the executable that `uvx` would
otherwise invoke. The new issue contributes a broader formulation, possible metadata shapes, and a
recent interoperability example, but those details can be centralized on the existing request.

Absent that existing issue, this would be an enhancement rather than a bug. Current uv behavior is
implemented around installed executable entry points, and choosing a module or callable from new
project metadata would add functionality. The source still has a TODO to determine the executable
from package entry points, and it deliberately reports a failure when the requested command has no
provider and `--from` was not used.

One normalization detail is narrower than the report suggests. The current implementation preserves
the verbatim spelling of a named requirement when choosing the executable, and the
`tool_run_verbatim_name` integration test verifies that `uvx` can run `change_wheel_version` even
though the distribution name normalizes to `change-wheel-version`. This does not solve the central
case of a package that advertises only a pipx-specific runner or no console executable.

## Related

- astral-sh/uv#3878 — Open canonical request. It asks uv to support the pipx workaround for a
  package that is intended to run with `python -m` and does not install the expected executable;
  maintainer discussion identifies the package's pipx-specific entry point.
- astral-sh/uv#7552 — Open adjacent caller-side proposal. It would let an invoker explicitly select
  a module with a shorter `uvx` syntax, whereas astral-sh/uv#21135 asks the project to declare the
  target once.
- astral-sh/uv#12976 — Open adjacent executable-selection request. It discusses automatically using
  a differently named console entry point, especially when a package provides only one; it does not
  define project metadata or cover a package with no executable entry point.
- astral-sh/uv#17779 — Open pull request that proposes automatically running a package's sole
  differently named executable. It addresses one executable-name mismatch, but not `[pipx.run]`, a
  module/callable declaration, or the zero-executable case.

## Supporting evidence

Literal searches covered `[pipx.run]`, `python -m`, `__main__.py`, `module:callable`, and the
no-executables diagnostic. Conceptual searches covered custom/default tool entry points, advertised
run targets, package-versus-executable naming, `--from`, and uvx module support. Open and closed
issues and open, closed, and merged pull requests were considered.

Fix-oriented inspection included astral-sh/uv#11603, which established the current executable
provider checks, and the current `crates/uv/src/commands/tool/run.rs` and
`crates/uv/tests/tool/tool_run.rs` behavior. The merged astral-sh/uv#7754 was a plausible module-run
fix but was ruled out because it implements `uv run -m`, not module selection for `uvx`. The external
pypa/pipx#2004 report was inspected as a lead; it confirms the current pipx uv-backend motivation,
but the repository-side capability is already represented by astral-sh/uv#3878.
