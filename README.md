# `implementation_name` filter doesn't work in `environment`

Issue: astral-sh/uv#23270

Classification: enhancement

## Summary

The reporter uses uv 0.13.8 on Ubuntu 26.04 and starts a project with `uv init --python 3.11 .`, then adds NumPy. After adding three disjoint `tool.uv.environments` entries for CPython on macOS ARM64, Linux x86_64, and Windows AMD64, they run `uv lock` and inspect the lockfile diff. Intel macOS wheels disappear, but PyPy wheels remain, including `numpy-2.4.6-pp311-pypy311_pp73-macosx_11_0_arm64.whl`.

The requested capability is implementation-aware pruning of wheel entries. The report demonstrates surplus artifact references; it does not demonstrate resolution of excluded PyPy dependency branches or installation of a PyPy wheel into CPython.

No exact duplicate found. astral-sh/uv#9473 confirms PyPy can be excluded from resolution; astral-sh/uv#15060 explains conservative wheel retention. astral-sh/uv#6512, astral-sh/uv#6961, astral-sh/uv#10571, and astral-sh/uv#10584 document earlier platform and architecture pruning enhancements.

## Draft response

Your `implementation_name == 'cpython'` constraint is valid for limiting dependency resolution. The lockfile wheel list is pruned separately: that code currently checks Python versions, operating systems, and architectures, but does not filter by Python implementation. That explains why PyPy wheels can remain in `uv.lock`; their presence does not mean uv will install them into CPython.

As discussed in astral-sh/uv#15060, retaining extra wheels is currently considered a pruning optimization gap. We can track implementation-aware wheel pruning here as an enhancement. The next step is to extend that filtering and cover your CPython-only configuration with a focused test.

## Classification

The demonstrated behavior is surplus artifact entries, with no evidence that an excluded environment is resolved or an incompatible wheel is installed. Documentation defines environments as a resolution constraint, and maintainers explicitly regard conservative retention of extra wheels as acceptable. Source confirms that uv.lock pruning checks Requires-Python and platform/architecture tags but does not compare implementation markers. Adding that pruning dimension is an improvement analogous to astral-sh/uv#10571, not an established resolution correctness failure. No exact duplicate or historical implementation-pruning fix was found, so a regression is not established.

The issue currently carries the bug label; the recommended classification follows the demonstrated effect and the explicit maintainer distinction between resolution correctness and optional wheel-list pruning. This recommendation is not based merely on explaining away a correctness failure: no failure of the resolution constraint or wheel compatibility selection has been established. A separate reproduction showing either would need assessment as a bug.

The historical platform and architecture changes predate uv 0.13.8. Their scope does not establish that implementation pruning ever worked, so this report is not established as a regression.

## Related

- astral-sh/uv#15060 (issue, open) — `platform_machine` environment marker not handled correctly. A maintainer explicitly describes retaining extra lockfile wheels as an acceptable, incomplete pruning optimization. Its main correctness report concerns missing Windows wheels and interpreter bitness; its surplus-wheel examples concern architectures, not Python implementations.
- astral-sh/uv#9473 (issue, closed) — Is there a way to specify that you don't care about pypy when creating a lockfile?. Maintainers recommended implementation_name in tool.uv.environments to exclude PyPy, and the reporter confirmed it resolved a PyPy-only dependency conflict. This establishes the resolution capability, but does not track pruning PyPy wheel entries.
- astral-sh/uv#6512 (issue, closed) — Restricting universal lock by OS still downloads wheels from all OSes. Historical enhancement for surplus uv.lock wheel entries despite restricted environments. Maintainers distinguished wheel-list pruning from installation and linked the platform-pruning implementation. Its scope was operating systems, not implementation_name.
- astral-sh/uv#6961 (pull request, merged) — Prune unreachable wheels from lockfile. Merged September 4, 2024; introduced pruning of wheels that mismatch platform-specific markers. It explains the existing platform filtering, but does not establish that filtering by Python implementation was implemented and later regressed.
- astral-sh/uv#10571 (issue, closed) — Filter out x86 wheels when `platform_machine == 'arm64'` is specified. Labeled enhancement, this requested extending wheel pruning from operating systems to architectures. It is a close precedent for extending the pruning dimensions again, but addresses a different wheel-tag component.
- astral-sh/uv#10584 (pull request, merged) — Filter wheels from lockfile based on architecture. Merged January 14, 2025; implemented astral-sh/uv#10571 by extending post-resolution wheel filtering to architectures. It does not filter CPython versus PyPy, so it is adjacent history rather than a fix for this report.

## Supporting evidence

- `docs/concepts/resolution.md:142` describes limited resolution environments. Its CPython example at line 161 uses exactly `implementation_name == 'cpython'` to avoid solving for other implementations. The setting is valid; the documentation describes the scope of dependency resolution.
- `crates/uv-lock/src/lock/mod.rs:2540` combines each distribution's marker with supported environments and calls `is_wheel_unreachable_for_marker` with no concrete interpreter tags. Thus the configured environments reach the pruning stage.
- `crates/uv-lock/src/lock/mod.rs:10407` defines that pruning function. It checks wheel variants, optional concrete tags, and `Requires-Python`, then filters operating-system and architecture tags. The function contains no comparison against `implementation_name` or `platform_python_implementation`. This explains the reported difference between platform and implementation filtering.
- In astral-sh/uv#15060, maintainer charliermarsh states: “Wheel filtering is considered an optimization, in that it's acceptable (though not optimal) to include ‘too many’ wheels. Filtering out necessary wheels should be considered a bug, though.” The relevant comment is dated August 5, 2025.
- In astral-sh/uv#9473, maintainer zanieb recommends `environments = ["implementation_name != 'pypy'"]`, and the reporter confirms it resolves their dependency conflict. This supports resolution filtering independently of the artifact list.
- astral-sh/uv#6512 and astral-sh/uv#10571 are labeled enhancement and supply the closest historical precedents for adding another pruning dimension.

The checked-out uv crate declares version 0.13.8. Findings above are based on source, documentation, and GitHub discussions; the NumPy command sequence was not executed. No build or tests were needed for this read-only assessment.

## Search scope and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests with authenticated gh. Literal searches covered implementation_name, pp311, PyPy wheels/lockfile/environments, and platform_python_implementation; conceptual searches covered environment markers, surplus wheels, filtering, pruning, and unreachable artifacts, without incidental NumPy versions or platforms. Inspected candidate bodies and comments and followed closure/reference chains to historical pruning PRs. No exact duplicate or prior implementation-pruning fix was found. Ruled out astral-sh/uv#18428 and astral-sh/uv#18445 because their fixed failure requires conflicts and concerns platform-marker propagation; astral-sh/uv#11463 concerns metadata downloads before pruning, and astral-sh/uv#14004 concerns custom wheel-name/libc constraints.

Additional inspected candidates were astral-sh/uv#17313, which follows the architecture discussion in astral-sh/uv#15060, and astral-sh/uv#13413, which concerns non-universal `uv pip compile` output in `pylock.toml`. Neither tracks implementation-based pruning in `uv.lock`.

The conflict-specific historical fix in astral-sh/uv#18445 merged March 13, 2026. Its explicit intersection with supported environments is present in the current source, and the new reproduction has no conflicts. It is therefore not evidence of the same regression.

## Suggested next step

Scope the issue to implementation-aware pruning of `uv.lock` wheel entries, using the reporter's CPython-only environments and a distribution with both CPython and PyPy wheels. Keep resolution filtering and artifact pruning distinct when validating a future change.
