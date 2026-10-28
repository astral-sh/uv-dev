# Add a flag / env var to ignore required-version

Issue: astral-sh/uv#22967

Classification: enhancement

## Summary

The reporter wants a global flag such as `--no-required-version` or
`--ignore-required-version`, plus an environment variable such as
`UV_NO_REQUIRED_VERSION=1`, to bypass only uv's version requirement while retaining
the rest of the project configuration. A warning could identify the ignored
constraint and running version.

The example pins `required-version = "==0.13.5"` while the developer runs uv 0.13.6.
The reporter describes failures in `uv run ruff check .`, `uv sync --frozen`, and
even `uv help`. Package-manager upgrades, particularly Homebrew, can create the
mismatch while CI uses the pin through setup-uv's `version-file`. The motivation
also includes testing a newer uv before changing the project's pin.

The existing workarounds have different tradeoffs: `--no-config` disables
configuration discovery broadly; manually invoking a pinned uv through `uvx`
requires selecting the version. The proposed override should keep settings such
as package behavior, default groups, constraints, and indexes.

The closest discussion is astral-sh/uv#11065 and its unmerged, closed implementation astral-sh/uv#11869; both concern automatic version switching. astral-sh/uv#10355 and astral-sh/uv#10374 cover a narrower, already-fixed uv self exception. No existing selective-override request was found.

## Draft response

Your requested override would be a separate enhancement: `required-version` currently rejects mismatches, and `--no-config` disables configuration discovery broadly. astral-sh/uv#11065 would instead run a compatible uv version; astral-sh/uv#11869 was closed without merging.

The next step is a design decision on allowing this opt-out and its warning behavior before implementation. For now, `uvx uv@0.13.5 ...` runs the pinned version with the project configuration. For help alone, `uv --help` is handled before the version check.

## Classification

Requests a new flag/environment override that skips only uv's version constraint while preserving other configuration. Rejecting an exact-pin mismatch is the implemented purpose of required-version. Existing automatic-version-selection discussions require using a compatible executable and do not cover deliberately testing the currently installed newer version. The historical uv self exception remains present, so this report does not establish a regression of that fix.

The reported `uv help` failure is consistent with the current source: the help
subcommand is dispatched after the global version check. The short `--help`
flag follows Clap's help action during argument parsing. The issue's requested
capability is a selective override for normal project commands; no prior
implementation of that capability was identified. Automatically choosing a
compatible executable would not support the stated goal of testing a newer
installed version outside the pin.

## Related

- astral-sh/uv#11065 — Automatically install and use a matching uv version to satisfy `requires-version`. (issue, open). Same version-mismatch error and manual uv tool run workaround. Requests automatically running a compatible uv version, whereas this report requests running the installed, incompatible version while retaining configuration. Labeled enhancement and wish; the maintainer said it was not a priority.
- astral-sh/uv#11869 — Auto-exec compatible uv version when required-version doesn't match (pull request, closed). Attempted to implement astral-sh/uv#11065 by delegating to uv tool run. Closed as stale on 2025-05-27 without merging. It would satisfy the version requirement, not provide the requested opt-out.
- astral-sh/uv#10355 — `uv self update` is blocked by `required-version` (issue, closed). Historical command-blocking bug caused by the same version check. Fixed specifically for uv self commands; it did not request a general override for uv run, uv sync, or uv help.
- astral-sh/uv#10374 — Avoid enforcing project-level required version for `uv self` (pull request, merged). Merged on 2025-01-07 to fix astral-sh/uv#10355 by excluding local configuration from uv self commands. That exception remains in the checkout and does not implement selectively bypassing required-version while retaining project settings.

## Supporting evidence

- `crates/uv-settings/src/settings.rs:260` describes `required-version` as an
  enforced requirement that causes uv to exit with an error on mismatch.
- `crates/uv-cli/src/settings.rs:113` derives the required version from filesystem
  configuration; there is no dedicated override in that resolution path.
- `crates/uv/src/lib.rs:313` selects filesystem configuration. At line 323,
  disabled configuration discovery produces no filesystem options. At line 325,
  `uv tool` and `uv self` use user/system configuration instead of local project
  configuration. The historical self-update fix remains present.
- `crates/uv/src/lib.rs:552` checks the running uv against the required version
  and returns an error on mismatch. `Commands::Help` is dispatched later at
  line 705, confirming why the help subcommand encounters this check.
- `crates/uv-cli/src/lib.rs:111` documents the broad effect of `--no-config`;
  line 123 declares `--help` with Clap's `HelpShort` action.
- In astral-sh/uv#11065, the maintainer described automatic matching-version
  execution as something they could not prioritize. Its enhancement/wish labels
  support describing it as a pending feature, without promising implementation.
  In astral-sh/uv#11869, the final maintainer comment explicitly closed the PR as
  stale; its merge timestamp is absent.
- The diff of astral-sh/uv#10374 adds `Commands::Self_` to the commands that
  ignore local configuration. It does not add any selective version override.
- In astral-sh/uv#13785, a maintainer explains that `required-version` checks
  the invoked uv executable and does not constrain package resolution or
  installation. That report was resolved by avoiding activation of an
  environment containing a different, dependency-installed uv.

## Search scope and candidate assessment

Searched astral-sh/uv with authenticated gh across open and closed issues and open, closed, and merged pull requests. Separated selective version-check bypass, configuration retention, help-command blocking, and pinned-version/package-manager workflows. Literal searches covered required-version, the Required uv version / does not match error, proposed flag and environment names, uv help, no-config, Homebrew, and version-file. Conceptual searches covered ignoring/overriding/bypassing version checks, version constraints, automatic matching-version execution, self-update, and wrappers, informed by configuration/CLI labels and maintainer comments. Followed comments and reference chains, including astral-sh/uv#12787 to astral-sh/uv#11065 and historical fixes. Ruled out astral-sh/uv#10547 and astral-sh/uv#16014 as duplicates because they request changing the installed version; astral-sh/uv#6662 requests bootstrapping via a wrapper. Also ruled out astral-sh/uv#13785 (a dependency-installed uv shadowing the CI installation) and astral-sh/uv#17609 / astral-sh/uv#18977 (configuration parsing masking version errors). No existing selective-override request was found. Some supplemental queries were rate-limited; PR keyword searches returned no results, so relevant PRs were inspected directly through issue references.

The initial report was decomposed before searching into the requested selective
override and optional warning, configuration retention, help-command blocking,
and externally upgraded local uv versus pinned CI. Searches for observable
behavior were kept distinct from searches for alternative solutions and prior
fixes. Candidate comparison used commands, desired behavior, configuration
handling, and merge status rather than the particular patch versions.

The following inspected candidates provide context but do not centralize this
request:

- astral-sh/uv#10547 and astral-sh/uv#16014 ask `uv self update` to select a version
  satisfying the requirement. Discussion in the latter points to automatic
  version switching and identifies the former as the existing request.
- astral-sh/uv#12787 was closed as a duplicate of astral-sh/uv#11065, confirming the
  canonical discussion for automatic bootstrapping.
- astral-sh/uv#6662 discusses a project wrapper that installs and runs a selected
  uv version, including local/CI consistency and multiple-project use cases.
- astral-sh/uv#8605, reached through astral-sh/uv#8823 and astral-sh/uv#8636, concerns
  introducing enforcement itself, including exact pins for CI consistency.
- astral-sh/uv#13785 shares the mismatch error and setup-uv context, but the
  confirmed trigger was a different uv executable installed transitively in an
  activated project environment.
- astral-sh/uv#17609 and merged astral-sh/uv#18977 concern reporting a version
  mismatch when unsupported settings otherwise cause a configuration parse
  warning/error. Their April 2026 fix strengthens enforcement and does not add
  the requested opt-out.

## Validation and limits

Findings are based on the issue, candidate discussions and timelines, PR metadata,
the historical self-update diff, and checkout source inspection. No runtime
reproduction or test execution was performed. Supplemental search rate limits
and empty PR keyword results limit claims of exhaustive coverage; directly
inspected PR states and discussions were available.

Only this temporary handoff README was authored. No GitHub changes were made.
The checkout already had a modified `agents/codex/config.toml` and an untracked
`.issue-triage-event.json`; neither was changed.
