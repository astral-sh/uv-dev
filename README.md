# Clarify the relation between --no-sync and other options that affects the sync

Issue: astral-sh/uv#21938

Classification: enhancement

## Summary

The reporter asks for clearer CLI documentation about how `uv run --no-sync` interacts with dependency-selection options such as `--no-dev`. Their example first installs the default development group with `uv sync`, then observes that `uv run --no-dev <binary_from_dev_group>` can still execute that already-installed binary.

Two established behaviors explain the result:

1. `--no-sync` skips updating the project environment, so selection flags cannot change the packages already installed there.
2. Without `--no-sync`, `uv run` uses an inexact sync by default. `--no-dev` excludes the `dev` group from the desired sync set, but an inexact sync does not remove an already-installed development package. `--exact` is the option that requests removal of packages outside the selected set.

The issue requests that `--no-dev` and other selection options describe this relationship more explicitly. No existing issue or pull request was found that tracks that broader CLI-help clarification.

The issue's example does not actually pass `--no-sync`: it runs `uv sync` followed by `uv run --no-dev`. It therefore demonstrates the separate inexact-sync behavior, not the effect of combining `--no-dev` with `--no-sync`. The reporter's expectation for the combined flags remains unclear.

## Maintainer follow-up

A maintainer questioned whether documenting the interaction on every sync-affecting option is feasible, noting that many such options necessarily have no effect on the base environment when `--no-sync` is used and that repeating this could make the CLI documentation excessively verbose. They asked what the reporter expected `--no-dev` to do when combined with `--no-sync`.

The next useful clarification is whether the request concerns:

1. `uv run --no-sync --no-dev`, where no base-environment synchronization occurs; or
2. the provided `uv run --no-dev` example, where an inexact sync excludes the group from the desired set but retains its already-installed packages.

No maintainer decision to implement or close the enhancement has been made. A narrower documentation change, such as explaining the relationship once on `--no-sync` or in shared command documentation, has not been proposed or accepted in the discussion.

## Classification

This is an enhancement. The source and maintainer comments establish that the observed behavior is intentional: `--no-sync` bypasses the base project-environment update, and ordinary `uv run` synchronization is inexact. The request is to improve the existing CLI descriptions so users understand that dependency-selection flags govern the desired sync set rather than independently removing or disabling installed packages.

The report is not a bug because no incorrect behavior is established. It is not primarily a support question because it proposes a concrete documentation improvement. It is not a duplicate: the closest prior discussions explain the behavior and one merged pull request documents inexact syncing, but none tracks clarification across the affected CLI flags. The maintainer follow-up lowers confidence that the requested across-the-board documentation expansion will be accepted and makes the reporter's expected behavior a prerequisite for evaluating a narrower change.

## Related

- astral-sh/uv#7914 (closed), “uv run ignores --no-dev” — This is the closest reproduction: a development executable installed earlier remains runnable under `uv run --no-dev`. A maintainer explained that `uv run` does not perform a strict sync and therefore does not uninstall the package.
- astral-sh/uv#14230 (closed), “uv run does not remove extraneous packages” — This is the broader canonical discussion of retained packages. Maintainers confirmed that inexact synchronization is intentional and identified `--exact` or `--isolated` for stricter behavior.
- astral-sh/uv#17366 (merged), “Clarify that `uv run` uses inexact syncing by default” — This resolved astral-sh/uv#14230 by documenting the exact-versus-inexact distinction that explains the reporter's `--no-dev` result.
- astral-sh/uv#7165 (closed), “Opt-into / opt-out of automatic re-sync with `uv run`?” — This is the design discussion that established `uv run --no-sync` as the way to run in the project environment without modifying it.
- astral-sh/uv#7192 (merged), “Add `uv run --no-sync`” — This implemented `--no-sync`; its summary states that the command runs in the project environment without locking or syncing.

## Supporting evidence

- `crates/uv-cli/src/lib.rs` describes `uv run --no-sync` as avoiding synchronization and implying `--frozen` because the project dependencies are ignored when the environment will not be synced.
- `crates/uv/src/commands/project/run.rs` branches on `no_sync` and skips project-environment synchronization. The implementation separately notes that `--with` requirements may still be layered over the base environment.
- `docs/concepts/projects/sync.md` states that `uv run` uses inexact syncing by default, ensuring required packages are installed without removing extraneous packages, and documents `uv run --exact` for exact syncing.
- astral-sh/uv#12558 and astral-sh/uv#16071 cover the adjacent inverse case: after `uv sync --no-dev`, a later plain `uv run` can install the default `dev` group because commands do not remember the flags passed to an earlier sync.

## Search scope

Searched this repository's open and closed issues and open, closed, and merged pull requests using exact combinations and phrases around `--no-sync`, `--no-dev`, `--extra`, `--group`, “Disable the development dependency group,” ignored options, and `uv run`. Conceptual searches covered automatic re-sync, dependency-group selection, retained or extraneous packages, exact versus inexact sync, stateless commands, and documentation. Candidate comments, timelines, linked fixes, current CLI definitions, the run implementation, and project sync documentation were inspected.

astral-sh/uv#17023 looked especially plausible because it combined `--no-sync`, group flags, and unexpected development dependencies, but it was ruled out after the reporter identified a stale container image as the cause. astral-sh/uv#12558 and astral-sh/uv#16071 are adjacent rather than equivalent because they concern a subsequent plain `uv run` re-adding development dependencies after a prior `uv sync --no-dev`.
