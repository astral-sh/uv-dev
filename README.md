# .env parsing - dotenvy fails to correctly parse files with spaces and some special characters

Issue: astral-sh/uv#23226

## Summary

The reporter describes application breakage when migrating from python-dotenv to uv's command-line dotenv loading with uv 0.13.6 and Python 3.14.10 on macOS and Linux. Dollar substitution, references containing underscores, and unquoted spaces produce different results. The reporter primarily requests documentation of these differences rather than replacing dotenvy.

All three concrete behaviors were reproduced with the installed uv 0.13.7 on Linux using Python 3.12.3 and synthetic inputs. A separate comparison with python-dotenv 1.0.1 confirms the compatibility differences. This is runtime evidence, replacing the earlier source-only assessment. The reported uv 0.13.6, Python 3.14.10, and macOS environment were not executed, and no regression between uv versions has been established.

Dollar substitution is an intentional dotenv feature; reproduction alone does not establish that all reported behavior is incorrect. Unquoted spaces produce a visible warning, while successful substitutions can change values without warning. The underscore-reference behavior remains a distinct upstream concern.

## Classification

Retain the existing bug classification for the reported parsing compatibility problem, including the underscore-reference concern tracked in allan2/dotenvy#170. The requested immediate action is documentation. Distinguish deliberate substitution semantics and whitespace restrictions from the surprising handling of underscore-containing references. No previously fixed uv regression or canonical uv duplicate was established by the existing issue-context investigation.

## Reproduction

**Outcome: reproducible.**

### Environment and isolation

- Installed executable on PATH: `/opt/hostedtoolcache/uv/0.13.7/x86_64/uv`; version output: `uv 0.13.7 (x86_64-unknown-linux-gnu)`.
- Runtime: CPython 3.12.3; Linux 7.0.0-1017-azure, x86_64 GNU/Linux.
- Reported environment: uv 0.13.6, Python 3.14.10, Darwin 25.6.0 arm64 and Linux 5.14.0-687.42.1.el9_8.x86_64. These exact combinations were not tested.
- Fixtures, captured results, package comparison, home/config/data directories, temporary files, and uv cache are under `/tmp/uv-23226-sgnp7ojv`.
- Each reproduction subprocess received an explicit environment containing PATH, temporary HOME/TMPDIR/XDG locations, temporary uv cache/Python locations, `UV_PYTHON_DOWNLOADS=never`, `PYTHONDONTWRITEBYTECODE=1`, and `LANG=C.UTF-8`. No parent dotenv variables or uv configuration were inherited. Only explicitly named synthetic variables were printed.
- Commands used `--no-config --offline run --no-project --python <explicit interpreter>` so no project, workspace, dependency group, or network resolution was involved. No application dependencies are needed to demonstrate parsing.

### Minimal fixtures and commands

The following reconstructs the four primary fixtures in a new temporary directory. Single-quoted heredoc delimiters keep the shell from expanding dollar signs.

```sh
repro_dir=$(mktemp -d /tmp/uv-23226-example-XXXXXX)
uv_executable=$(command -v uv)
python_executable=$(command -v python3)
cd "$repro_dir"
mkdir home tmp config data cache python

cat > show.py <<'PY'
import json
import os
import sys

print(json.dumps({key: os.environ.get(key) for key in sys.argv[1:]}))
PY

cat > dollar.env <<'ENV'
DOLLAR=prefix$MISSING
ENV

cat > underscore.env <<'ENV'
A=short
A_PLAIN=whole
BARE=$A_PLAIN
BRACED=${A_PLAIN}
ENV

cat > spaces.env <<'ENV'
BEFORE=loaded
SPACES=foo bar
AFTER=loaded
ENV

cat > quoted.env <<'ENV'
SPACES="foo bar"
ESCAPED=foo\ bar
AFTER=loaded
ENV

# Start an isolated shell for the explicit invocations below.
env -i PATH="$PATH" HOME="$repro_dir/home" TMPDIR="$repro_dir/tmp" \
  XDG_CONFIG_HOME="$repro_dir/config" XDG_DATA_HOME="$repro_dir/data" \
  XDG_CACHE_HOME="$repro_dir/cache" UV_CACHE_DIR="$repro_dir/cache/uv" \
  UV_PYTHON_INSTALL_DIR="$repro_dir/python" UV_PYTHON_DOWNLOADS=never \
  PYTHONDONTWRITEBYTECODE=1 LANG=C.UTF-8 \
  UV_REPRO_EXECUTABLE="$uv_executable" UV_REPRO_PYTHON="$python_executable" \
  /bin/sh <<'SH'
"$UV_REPRO_EXECUTABLE" --no-config --offline run --no-project --python "$UV_REPRO_PYTHON" --env-file dollar.env show.py DOLLAR
"$UV_REPRO_EXECUTABLE" --no-config --offline run --no-project --python "$UV_REPRO_PYTHON" --env-file underscore.env show.py A A_PLAIN BARE BRACED
"$UV_REPRO_EXECUTABLE" --no-config --offline run --no-project --python "$UV_REPRO_PYTHON" --env-file spaces.env show.py BEFORE SPACES AFTER
"$UV_REPRO_EXECUTABLE" --no-config --offline run --no-project --python "$UV_REPRO_PYTHON" --env-file quoted.env show.py SPACES ESCAPED AFTER
SH
```

Observed results from the equivalent explicit subprocess invocations:

| Fixture | stdout | stderr |
| --- | --- | --- |
| `dollar.env` | `{"DOLLAR": "prefix"}` | Empty |
| `underscore.env` | `{"A": "short", "A_PLAIN": "whole", "BARE": "short_PLAIN", "BRACED": "whole"}` | Empty |
| `spaces.env` | `{"BEFORE": "loaded", "SPACES": null, "AFTER": null}` | `warning: Failed to parse environment file ` followed by the filename and `at position 4: foo bar` |
| `quoted.env` | `{"SPACES": "foo bar", "ESCAPED": "foo bar", "AFTER": "loaded"}` | Empty |

All commands exited 0. Undefined `$MISSING` disappears from the unquoted dollar value. Bare `$A_PLAIN` substitutes `A` and leaves `_PLAIN` literal, while `${A_PLAIN}` selects the complete name. An invalid unquoted space leaves earlier variables available and skips both the invalid variable and the remainder of that file. Quoting or escaping the space works.

Two additional explicit controls were run with the same command shape and isolation:

- `literal.env`, containing `SINGLE='prefix$MISSING'` and `DOUBLE="prefix${MISSING}"`, produced `{"SINGLE": "prefix$MISSING", "DOUBLE": "prefix"}` without a warning. Single quotes retain literal dollar signs; double quotes with braced references still substitute.
- `double-unbraced.env`, containing `DOUBLE="prefix$MISSING"`, produced `{"DOUBLE": null}` and `warning: Failed to parse environment file ` followed by the filename and `at position 15: "prefix$MISSING"`. It also exited 0. Double quoting alone is therefore not a general workaround for dollar-containing input.

### Comparison with python-dotenv

Using the installed uv executable, python-dotenv 1.0.1 was installed with `uv --no-config pip install --python <explicit interpreter> --target /tmp/uv-23226-sgnp7ojv/comparison --index-url https://pypi.org/simple --only-binary :all: python-dotenv==1.0.1`, under the same temporary environment. A separate isolated Python subprocess imported that temporary package and called `dotenv_values` on the same files. It exited 0 without stderr.

- `dollar.env`: `DOLLAR` was `prefix$MISSING`, retaining the unbraced reference.
- `underscore.env`: `BARE` was the literal `$A_PLAIN`; `BRACED` was `whole`. python-dotenv does not interpret this bare reference as a full-name substitution either.
- `spaces.env`: all three assignments loaded, including `SPACES` as `foo bar` and `AFTER` as `loaded`.
- `literal.env`: results matched uv for these particular single-quoted and double-quoted braced-reference inputs.
- `double-unbraced.env`: `DOUBLE` was `prefix$MISSING`, with no parse warning.

The reporter did not specify a python-dotenv version; the comparison establishes these examples for 1.0.1, not universal compatibility across dotenv implementations or versions.

### Existing test coverage and implementation

The relevant existing tests were read, including their fixtures and snapshots:

- `crates/uv/tests/project/run.rs::run_with_env` and `run_with_env_file` check opt-in loading and ordinary underscore-containing keys with literal values. They do not test references containing underscores.
- `crates/uv/tests/project/run.rs::run_with_malformed_env` writes `THE_^EMPIRE_VARIABLE=darth_vader` and snapshots a parse warning, an unset queried variable, and exit 0. It covers malformed-key handling, not embedded spaces, dollar substitution, or stopping after an invalid middle assignment.
- `crates/uv/tests/project/run.rs::run_with_multiple_env_files` covers file ordering; `run_with_env_omitted` covers disabling loading. Neither tests these syntax cases.
- `crates/uv/tests/tool/tool_run.rs::run_with_env_file` creates a local executable project and loads literal dotenv assignments. It does not exercise these syntax cases.
- At initial reproduction, searches in `crates/uv/tests/` and `crates/uv-client/tests/it/` found no targeted integration coverage for the three reported syntax cases. The parent regression contribution, astral-sh/uv-dev#3126, subsequently added `run_with_env_file_unset_substitution`, `run_with_env_file_underscore_substitution`, and `run_with_env_file_unquoted_spaces` in `crates/uv/tests/project/run.rs`. Their snapshots assert the reproduced behavior: `prefix`, `short_PLAIN` versus `whole`, and a warning with partial loading, respectively. These tests were read during the fix investigation and left unchanged. The project `run` test module is gated on `test-python` and `test-pypi` in `crates/uv/tests/project/main.rs`.

The checkout pins dotenvy 0.15.7 in `Cargo.toml` and `Cargo.lock`. `crates/uv-command-support/src/child.rs::read_env_files` calls `dotenvy::from_path_iter`, warns on a parse error, and breaks iteration for that file. This agrees with the observed partial loading; the runtime conclusion does not depend on source inspection alone. The existing handoff also checked the released uv 0.13.6 lockfile and found dotenvy 0.15.7. Local release notes for uv 0.13.6 and 0.13.7 list no dotenv parser fix. The report concerns migration between implementations, with no last known-good uv version supplied.

## Fix

**Outcome: not fixed; checkout unchanged.**

The parent regression contribution is astral-sh/uv-dev#3126. Its three tests describe the existing dependency semantics, not one uv implementation error with an agreed replacement behavior. The report requests documentation and explicitly acknowledges intentional substitution behavior. Making unquoted `$MISSING` literal, resolving `$A_PLAIN` as a complete name, and accepting unquoted spaces would be separate syntax policy changes; python-dotenv 1.0.1 itself leaves the bare underscore reference literal rather than resolving it.

The root causes were confirmed against the published dotenvy 0.15.7 source, downloaded to `/tmp/uv-23226-fix-zaymmco0` and verified against its SHA-256 checksum in `Cargo.lock`:

- `parse_value` uses `is_alphanumeric()` to accumulate bare substitution names, so `_` terminates a reference. The upstream test `variable_without_parenthesis_is_substituted_before_separators` explicitly asserts this separator behavior.
- `apply_substitution` replaces unknown names with empty strings. The upstream test `substitute_undefined_variables_to_empty_string` explicitly asserts that behavior.
- `parse_value` enters `expecting_end` at unquoted whitespace and rejects subsequent non-comment text. Its substitution branch also consumes a terminating quote as output instead of closing the quote, explaining the already demonstrated double-quoted unbraced-reference failure.
- `from_path_iter` and `from_read_iter` expose the same parser without a configurable substitution policy. By the time uv receives an item, successful substitution has already discarded the original reference text. Changing uv's error loop cannot repair those values.

The affected uv integration is `crates/uv-command-support/src/child.rs::read_env_files`. Both `crates/uv-project-commands/src/run.rs` and `crates/uv-tool-commands/src/run.rs` call it; neither has a separate dotenv parser or producer requiring a distinct round-trip fix. The neighboring project loading, multiple-file, disabling, and malformed-key tests, plus `crates/uv/tests/tool/tool_run.rs::run_with_env_file`, were inspected. The tool fixture uses literal assignments and does not contradict the demonstrated parser behavior. No additional command-only test variants were added for this shared path.

A dependency patch belongs in dotenvy and would require a dependency/lockfile change, outside the allowed checkout paths. Reimplementing or preprocessing dotenv syntax in uv would introduce a new parsing mechanism that must account for quoting, escaping, multiline values, and substitution order, rather than a small correction to the shared reader. That approach needs a design decision and risks changing behavior beyond the confirmed examples. The requested documentation update is also outside the permitted paths. No speculative production workaround or changed test expectations were retained.

### Focused validation attempt

`cargo nextest` is unavailable. The debug-profile baseline command `cargo test --offline --locked -p uv --test project run_with_env_file_ -- --nocapture` stopped before compilation because the local crates.io cache lacks `anstream`. Thus the parent tests were inspected but their passing baseline was not independently verified in this fix session; no changed-expectation failure or successful debug end-to-end fix validation is claimed. No further build was pursued once the parser ownership and syntax-policy limitations ruled out an authorized focused fix. The successful installed-binary reproduction and python-dotenv comparison remain documented above; they are not validation of a production change.

`git diff --exit-code` and `git status --short` confirmed that the checkout remained clean. Only this README was revised. No tests were weakened or removed, and no GitHub or Git configuration changes were made.

## Documentation and maintainer next step

`docs/concepts/configuration-files.md`, under “Environment variable files,” names dotenvy, gives a quoted-space example, and documents file selection and precedence. It does not explicitly explain these substitution and whitespace differences.

A focused documentation update can explain quoting or escaping spaces, single quotes for literal dollar signs, `${A_PLAIN}` for names containing underscores, and the distinction between silent substitutions and parse warnings that stop loading a file while allowing the child command to run. Keep the underscore concern linked to its upstream tracker. The comparison should not claim that python-dotenv expands bare `$A_PLAIN` correctly: the tested version leaves it literal.

## Related

The following relationships are retained from the existing issue-context investigation; this reproduction did not change GitHub state or repeat its issue searches.

- astral-sh/uv-dev#3126 — Parent regression contribution adding the three current-behavior integration tests inspected during the fix investigation.
- astral-sh/uv#8862 — Closed dotenv support issue whose comments include an underscore-containing substitution failure. Closure concerned opt-in loading through astral-sh/uv#16926, not a parsing fix.
- allan2/dotenvy#170 — Open upstream concern about inconsistent substitution of names containing underscores. Braced references are a workaround for the reproduced case.
- allan2/dotenvy#93 — Dollar-containing values changing through substitution; closed as intended behavior, with documentation concerns.
- allan2/dotenvy#11 — Unquoted values containing spaces; upstream recommends quoting or escaping whitespace.
- allan2/dotenvy#19 — Documentation of differences among dotenv implementations, including quoting, escaping, spaces, and substitution.

The existing investigation excluded several superficially similar candidates: astral-sh/uv#9465 concerns environment precedence; astral-sh/uv#16107 was traced by its reporter to a VS Code extension; astral-sh/uv#15806, merged astral-sh/uv#15815, and astral-sh/uv-dev#156 concern spaces in env-file paths rather than values. astral-sh/uv#1384 and merged astral-sh/uv#8263 concern initial dotenv support; astral-sh/uv#16926, astral-sh/uv#9994, and astral-sh/uv#16672 concern automatic loading, configuration, or missing files. Upstream allan2/dotenvy#82 and allan2/dotenvy#139 concern adjacent escaping and shell-modifier syntax.

## Validation and scope

The original runtime reproduction used only synthetic values and temporary files. The subsequent fix investigation inspected the parent tests, the shared reader, both consumers, neighboring integration coverage, and the checksum-verified upstream parser. Its focused offline debug test attempt failed during dependency resolution before compilation. Only this issue-context README was updated; no checkout files, existing user state, Git configuration, or GitHub state were modified. No full test suite or release-profile command was run. The checkout was clean at the beginning and end of the fix investigation; the modified config and untracked event noted in the earlier reproduction session were not present in this checkout state.
