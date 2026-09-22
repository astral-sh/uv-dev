# uv pip install -r pylock.toml ignores PEP 751 `default-groups`

Issue: astral-sh/uv#21917

Classification: bug

## Summary

Installing the reported PEP 751 `pylock.toml` with `uv pip install -r pylock.toml` omits a package whose marker is enabled by a name present only in top-level `default-groups`. PEP 751 says the default `dependency_groups` marker environment should be created from `default-groups`, while those names should not normally be exposed in `dependency-groups`.

The current source confirms the gap. `PylockToml` deserializes `default_groups` and `dependency_groups` separately. The `pip install` and `pip sync` paths apply `lock.default_groups` as defaults, but then obtain the concrete marker groups by filtering only `lock.dependency_groups`. Consequently, a default-only name cannot reach PEP 751 marker evaluation. The nearby integration test does not cover this shape because it lists its `default` group in both fields.

No existing issue tracks this exact default-selection bug. The closest history is astral-sh/uv#14740 and its fix, astral-sh/uv#14755, which added parsing and evaluation for `dependency_groups` markers but did not cover a default group omitted from `dependency-groups`.

## Draft response

Thanks for the focused reproduction. This is a bug in the current pylock group-selection path, separate from astral-sh/uv#14740 and astral-sh/uv#14755. Those added parsing and evaluation for PEP 751 `dependency_groups` markers. The current `pip install` and `pip sync` paths read `default-groups`, but build the marker group list by enumerating only `dependency-groups`, so a default-only group is dropped before marker evaluation. Our existing test covers only the case where the default group is also declared in `dependency-groups`.

The next step is to add `pip install` and `pip sync` regression cases with a group present only in `default-groups`, then ensure those default names are passed to marker evaluation.

## Classification

This is a bug, not an enhancement or question. PEP 751 defines `default-groups` specifically as the default source for the `dependency_groups` marker environment and says these names should not normally be listed in `dependency-groups`. The source-backed filtering behavior prevents a valid default-only group from affecting marker evaluation and silently changes the installation result.

It is not a duplicate of astral-sh/uv#14740. That issue reported a parse error for the `dependency_groups` marker identifier, and astral-sh/uv#14755 added marker parsing and evaluation. astral-sh/uv#21917 reaches that evaluator but supplies the wrong default group set.

## Related

- astral-sh/uv#14740 (closed) — Closest adjacent issue. It reported that `uv pip install -r pylock.toml` rejected a PEP 751 `dependency_groups` marker during parsing. Maintainer comments confirmed marker extensions were unsupported at the time. The command and marker are the same, but the observed failure and mechanism differ: astral-sh/uv#21917 parses successfully and silently omits a default-selected package.
- astral-sh/uv#14755 (merged) — Fix for astral-sh/uv#14740. It added `extras` and `dependency_groups` marker evaluation to `uv pip install` and `uv pip sync`. Its patch also applied `default_groups` and then enumerated only declared `dependency_groups`, which explains why it covers declared groups but not the default-only PEP 751 shape reported here.

## Search scope and evidence

Searched open and closed issues and open, closed, and merged pull requests using literal terms including `default-groups`, `dependency_groups`, `dependency-groups`, `pylock`, `PEP 751`, and the `uv pip install` command. Conceptual searches covered group selection, marker evaluation, silently skipped packages, installation from lock files, and default dependency sets. Historical/fix searches covered the original PEP 751 tracker, initial pylock installer support, and the later marker-extension fix.

astral-sh/uv#12584 and astral-sh/uv#12992 were inspected but are broader initial PEP 751 support work rather than trackers for this narrow omission. astral-sh/uv#14005 was also inspected as a plausible `default-groups` result, but it concerns project/workspace scoping rather than PEP 751 pylock installation. No open issue or pull request was found that already tracks default-only pylock groups being lost before marker evaluation.

Supporting repository evidence:

- `crates/uv-lock/src/lock/export/pylock_toml.rs` stores `default_groups` separately from `dependency_groups`.
- `crates/uv/src/commands/pip/install.rs` and `crates/uv/src/commands/pip/sync.rs` apply `lock.default_groups`, then call `group_names(lock.dependency_groups.iter())` before `resolve_pylock_toml`.
- `crates/uv/tests/pip_install/pip_install.rs::pep_751_groups` declares `default` in both `dependency-groups` and `default-groups`, leaving the default-only form untested.
