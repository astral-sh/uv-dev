# Support `--env-file-if-exists` (no error thrown sans `.env` file)

Issue: astral-sh/uv#23323

Classification: duplicate

## Summary

The reporter proposes a Node-style `uv run --env-file-if-exists .env --script` option so the same
shebang, development script, or CI command loads `.env` when present and still runs when it is
absent. The missing-file behavior is already under discussion in astral-sh/uv#16672 and
astral-sh/uv#9270. An open PR proposes skipping missing files listed in configuration, not adding
an optional command-line flag.

## Draft response

`uv run --env-file` treats the named file as required. We're discussing opt-in loading that skips
missing files in astral-sh/uv#16672. Please add your shebang example there: it shows why a CLI flag
would be useful even if a config-file option were available. For now, a shell wrapper can pass
`--env-file .env` when the file exists and `--no-env-file` otherwise.

## Classification

The proposed flag gives a new spelling and a shebang use case for the same opt-in behavior already
requested in astral-sh/uv#16672 and astral-sh/uv#9270: load the named dotenv file if present, and run
without it if absent. Both older reports start from `UV_ENV_FILE`; the former discusses a separate
opt-in, and the latter also discusses a command-line flag. The existing strict behavior is covered
by a test, and the report does not describe a regression.

## Design feedback

`woodruffw` raised a concern about making a flag's effect conditional on another state, such as
whether a file exists. This is feedback on the proposed CLI design, not a decision against
optional dotenv loading. The maintainer asked for the concrete workflow and how it reaches the
missing-file failure.

The reporter clarified that the same executable script should run with an unchanged shebang both
locally and in GitHub Actions. Locally it would load an uncommitted `.env` file; in CI there is no
`.env`, and the workflow would supply the variables through its environment from repository
secrets. The flag name is negotiable; the requirement is to continue when the file is absent.
The shebang illustrates the requested behavior; no failing CI invocation was provided.

## CI workaround

BurntSushi suggested running `touch .env` in the CI job before launching the script. With the
existing `--env-file .env` in the shared shebang, creating an empty file at the path uv reads
avoids the missing-file error without adding dotenv values, so the script can use variables
supplied by CI. A script that checks whether `.env` exists will now find it, so this is not
equivalent to optional loading. The reporter has not confirmed the workaround in their workflow.

## Related

- astral-sh/uv#16672 (open) — “Do not error when no environment file is found.” Canonical discussion for best-effort dotenv loading in `uv run`. The report uses `UV_ENV_FILE`; a maintainer suggested a separate default-file setting rather than weakening explicitly requested files. The broader dotenv opt-in discussion points here for missing-file handling.
- astral-sh/uv#9270 (open) — “[Feature request] add option to allow UV_ENV_FILE to point to non-existing file.” Same missing-file failure when sharing a dotenv configuration across projects. The discussion includes an explicit flag to load `.env` only when present.
- astral-sh/uv#22385 (open PR) — “Add `env-file` setting for loading environment files in `uv run`.” The proposal skips absent files listed in configuration and explicitly leaves missing `--env-file` arguments as errors. It does not implement the requested CLI flag.

## Supporting evidence

`crates/uv-cli/src/lib.rs:3313` binds `--env-file` to `UV_ENV_FILE`.
`crates/uv-project-commands/src/run.rs:174` propagates the dotenv loader's error before executing
the script. `crates/uv-command-support/src/child.rs:19` maps `NotFound` to
`No environment file found at:`, and `crates/uv/tests/project/run.rs:5213` expects exit code 2 for
an explicitly named missing file. `docs/concepts/configuration-files.md:100` documents
`--no-env-file` as an override for dotenv loading.

## Search scope

Searched open and closed issues and open, closed, and merged PRs with `gh` for the proposed flag,
`--env-file`, `UV_ENV_FILE`, `No environment file found at`, and variants for optional or best-effort
dotenv loading and `uv run --script`. Followed comments and linked dotenv changes.
astral-sh/uv#8263 added explicit loading, not an optional CLI path. Ruled out astral-sh/uv#16926
(implicit discovery and persistent opt-in), astral-sh/uv#15806 (a path containing spaces), and
astral-sh/uv#18017 (an option passed after the script path).
