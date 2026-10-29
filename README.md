# `uvx` "Failed to spawn" on CPython dev builds with `+dev` version suffix

Issue: astral-sh/uv#22989

Classification: bug

## Summary

The reporter runs `uvx --python ./python.exe pycowsay hello` against locally built CPython. A Python 3.14.8+ build works, while Python 3.15 and 3.16 development builds using the +dev suffix fail with:

```text
error: Failed to spawn: pycowsay
  cause: No such file or directory (os error 2)
```

The report also names tox and reproductions on macOS 15 arm64 with uv 0.13.6 and Ubuntu 24.04 x86_64 with uv 0.12.23. The example failing interpreter is Python 3.16.0a1+dev. The issue had no comments at inspection.

The closest history is astral-sh/uv#1357 and its fix astral-sh/uv#1771. python/cpython#99968 confirms the intentional suffix change. astral-sh/uv#13992 and astral-sh/uv#20849 cover adjacent spawn diagnostics. No existing tracker covers the source-confirmed +dev metadata defect.

Source inspection identifies a metadata round-trip defect and a path from that defect to stale tool-script interpreter paths. This analysis did not execute the reporter's development interpreter, so the generated script remains the key confirmation for the reported run.

## Report decomposition

- Observable symptom: a tool invocation reaches process spawning and returns ENOENT; the expected behavior is execution of the installed tool.
- Commands and subsystems: uvx / uv tool run, explicit interpreter selection, cached tool environments, virtual-environment metadata, and installed console entry points.
- Trigger reported: CPython development versions ending in +dev, beginning with 3.15; older bare-+ builds work. The report covers two tools and two Unix platforms, so package and platform names were not treated as defining the problem.
- Exact search identifiers: Failed to spawn, No such file or directory, os error 2, python.exe, +dev, 3.16.0a1, and the upstream reference python/cpython#99968.
- Requested outcome: existing tool execution should work with those development interpreters; no separate new command or capability is requested.

## Draft response

The +dev suffix is intentional, as confirmed in python/cpython#99968. The source shows a bug in uv's environment metadata handling: it writes that version into pyvenv.cfg, but rejects the local-version suffix when reading it back. That parse failure makes the environment appear non-relocatable, which can leave tool scripts pointing at a temporary interpreter path after the environment moves into the cache.

This is distinct from the bare trailing-+ parsing fix in astral-sh/uv#1771. To confirm this explains your launch failure, could you share the output of `uvx -vv --python ./python.exe pycowsay hello` and the first few lines of the generated cached `pycowsay` script? The configuration round trip and relocatable entry-point handling need regression coverage.

## Classification

Source inspection confirms that uv can write a valid +dev Python version to pyvenv.cfg and then reject it when reading the same file. The swallowed parse error disables relocatable entry-point generation, which can leave stale interpreter paths after a tool environment is moved into the cache. This is incorrect behavior. No matching duplicate or demonstrated recurrence of the historical trailing-plus fix was found; the reporter's exact generated entry point has not been inspected.

The historical fix predates the May–June 2026 upstream suffix change and still exists in the checkout. There is no evidence that its handling of bare trailing plus signs stopped working. The current issue should therefore remain a bug about the newly exposed metadata behavior, rather than be closed into the historical parsing discussion or the separate diagnostic enhancement.

## Related

- astral-sh/uv#1357 — Support Python versions with trailing `+` (issue, closed). Historical CPython development-build compatibility issue: versions such as 3.11.4+ failed during interpreter version parsing. Maintainer discussion led to stripping the bare trailing plus. The new report reaches tool spawning with a valid +dev local version; the source exposes a different pyvenv.cfg parsing and relocation path.
- astral-sh/uv#1771 — strip trailing `+` from version number of local Python builds (pull request, merged). Merged on 2024-02-20 to fix astral-sh/uv#1357 by stripping a final + from python_full_version. That workaround remains in the checkout and deliberately leaves +dev intact. It does not fix rejection of local versions when reading pyvenv.cfg, so this report does not establish a regression of that fix.
- python/cpython#99968 — Python version ending with plus sign not compatible with PEP 508 `python_full_version` marker (issue, closed). Verified upstream context for the trigger. CPython maintainers confirmed on 2026-05-27 that upcoming 3.15+ development builds would use +dev; a June comment shows 3.15.0b2+dev. This establishes that the suffix is intentional, but does not track uv's tool-launch failure.
- astral-sh/uv#13992 — Improve error message on "failed to spawn" where the Python interpreter is missing (issue, open). Tracks the same misleading spawn/ENOENT symptom when an entry point exists but its shebang interpreter is missing. This is relevant to the stale interpreter path exposed by the source analysis, but requests better diagnostics rather than correcting +dev metadata parsing and tool-environment relocation.
- astral-sh/uv#20849 — Diagnose dangling shebang interpreters in `uv run` / `uv tool run` (pull request, open). Proposes diagnostics for astral-sh/uv#13992, including uv tool run. It explains how an existing script with a missing shebang interpreter yields ENOENT. The proposed change only adds a hint after spawning fails; it does not repair the +dev configuration parsing defect.

## Supporting source evidence

Inspected checkout: `d2152bc80fe9b9ef3fe6acceffd408d9bdd90b23`.

1. `crates/uv-python-interpreter/python/get_interpreter_info.py:65` obtains `python_full_version` from `platform.python_version()`; lines 98–99 strip only a bare final plus. A +dev suffix survives.
2. `crates/uv-virtualenv/src/virtualenv.rs:556` writes the full version into `version_info` for environments not using a managed minor-version link. The same function writes `relocatable = true` at line 573 when requested.
3. `crates/uv-python-interpreter/src/virtualenv.rs:143` parses `version` and `version_info` with `PythonVersion::from_str`. In `crates/uv-python-types/src/python_version.rs:49`, that parser rejects any local version. Under PEP 440, +dev is a local-version label, not a .dev prerelease segment.
4. `crates/uv-python-interpreter/src/environment.rs:287` implements relocatability as `self.cfg().is_ok_and(|cfg| cfg.is_relocatable())`. A configuration parse failure therefore returns false even if the file contains `relocatable = true`.
5. `crates/uv-installer/src/installer.rs:122` passes that result to wheel installation. `crates/uv-install-wheel/src/wheel.rs:979` keeps the absolute executable path when relocatability is false; line 128 formats the shebang.
6. `crates/uv-environment-operations/src/environment.rs:247` creates a relocatable temporary tool environment, installs into it, and subsequently persists it at the cache archive location. Absolute entry-point interpreter paths can consequently refer to the old temporary location.
7. `crates/uv-tool-commands/src/run.rs:432` prepends the environment's scripts directory to PATH; line 451 attempts to spawn the tool, and line 467 adds the reported Failed to spawn context.

The rejection of the metadata and fallback to non-relocatable installation are directly visible in source. Whether the reporter's particular script contains the resulting stale path needs confirmation. The generic ENOENT message alone cannot distinguish a missing command from a missing shebang interpreter.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues for the exact spawn error, python.exe, +dev, 3.15/3.16, 3.16.0a1, and 99968; expanded to development/self-built CPython, prereleases, local versions, sysconfig, pyvenv.cfg/version_info, relocatable environments, and shebangs. Kept symptom searches separate from source-derived causes. PR search/list queries returned empty results even for a known merged fix, so additionally searched titles and bodies through the complete paginated REST pull-request listing with state=all, and inspected linked PRs, comments, reviews, and timelines. Some supplemental search queries were rate-limited. Ruled out astral-sh/uv#15967 and astral-sh/uv#15987, fixed by astral-sh/uv#15990 in 0.8.20, because they require disabled caching; astral-sh/uv#13233 and merged astral-sh/uv#13234 concern prerelease environment invalidation under uv run --no-sync; astral-sh/uv#15692 concerns system-site-package visibility, not interpreter suffixes. No existing tracker for the +dev metadata/relocation defect was found.

Further inspected candidates were also distinct:

- astral-sh/uv#12000 reused an environment after a Docker Python-version change and raised ModuleNotFoundError; the reporter confirmed resolution after uv 0.7.14.
- astral-sh/uv#19920 concerned managed minor-version links and pre-commit's version health check. Maintainer comments identify shortened major/minor metadata, not local-version rejection.
- astral-sh/uv#11796 concerns invoking an unavailable versioned Python command; its comments point to diagnostic work in astral-sh/uv#12201.
- astral-sh/uv#7604 concerns pyenv shell configuration and interpreter selection. astral-sh/uv#16231 concerns copying a virtual environment between different Python patch versions. Neither establishes the +dev failure.

The authenticated GitHub CLI was used only for reads. Searches included historical closed issues and merged fixes as well as open work. The upstream lead was verified against its discussion rather than assumed to establish the uv failure's cause.

## Next steps and verification limits

Obtain verbose output from the failing command and inspect the cached tool entry point's opening lines and interpreter target. A targeted fix should ensure versions emitted into environment metadata can be read back, and that metadata parsing cannot silently defeat the relocatability needed for cached tool execution. Regression coverage should exercise a +dev interpreter and a console entry point after the environment is moved into the cache.

No build or integration reproduction was run. No checkout files or GitHub objects were changed. This README is the maintainer handoff; the reply above is a draft.
