# Clarify the relation between --no-sync and other options that affects the sync

Issue: astral-sh/uv#21938

Classification: enhancement

## Summary

The reporter asks for clearer CLI documentation about how `uv run --no-sync` interacts with dependency-selection options such as `--no-dev`. Their example first installs the default development group with `uv sync`, then observes that `uv run --no-dev <binary_from_dev_group>` can still execute that already-installed binary.

Two established behaviors explain the result:

1. `--no-sync` skips updating the project environment, so selection flags cannot change the packages already installed there.
2. Without `--no-sync`, `uv run` uses an inexact sync by default. `--no-dev` excludes the `dev` group from the desired sync set, but an inexact sync does not remove an already-installed development package. `--exact` is the option that requests removal of packages outside the selected set.

The issue requests that `--no-dev` and other selection options describe this relationship more explicitly. No existing issue or pull request was found that tracks that broader CLI-help clarification.

## Draft response

`--no-dev` selects a sync target that excludes the `dev` group; it does not remove or hide packages independently of synchronization. `uv run` uses an inexact sync by default, so a development executable already installed by `uv sync` can remain runnable, as discussed in astral-sh/uv#7914 and documented by astral-sh/uv#17366. With `--no-sync`, the project environment is not updated at all, so dependency-selection flags do not change what is already installed.

The current option text does not make those interactions clear. It would be reasonable to clarify the dependency-selection options and cross-reference `--no-sync` and `--exact`, while accounting for options such as `--with` that can still create an overlay.

## Classification

This is an enhancement. The source and maintainer comments establish that the observed behavior is intentional: `--no-sync` bypasses the base project-environment update, and ordinary `uv run` synchronization is inexact. The request is to improve the existing CLI descriptions so users understand that dependency-selection flags govern the desired sync set rather than independently removing or disabling installed packages.

The report is not a bug because no incorrect behavior is established. It is not primarily a support question because it proposes a concrete documentation improvement. It is not a duplicate: the closest prior discussions explain the behavior and one merged pull request documents inexact syncing, but none tracks clarification across the affected CLI flags.

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
