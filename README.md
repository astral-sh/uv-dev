# Error with venv activation command for ksh on OpenBSD

Issue: astral-sh/uv#22537

Classification: bug

## Summary

The reported activation-hint failure was reproduced with the installed uv 0.13.0 on Linux using oksh 7.9, the portable OpenBSD ksh. The checkout now fixes the hint by emitting `.` for ksh and retains an updated regression test from astral-sh/uv-dev#2622. Environment creation succeeds and prints `Activate with: source .venv/bin/activate`; executing that command fails with `ksh: source: not found` and exit status 127. The shell reports the same `KSH_VERSION` as the reporter.

A follow-up check also found that `. .venv/bin/activate` fails inside the generated script on this shell with a `${.sh.file}` bad substitution. Changing the printed hint to `.` addresses the unavailable command, but is not a verified complete activation workaround. Neither check was run on native OpenBSD.

## Report scope

- Reported platform: OpenBSD, `x86_64-unknown-openbsd`; the OpenBSD release was not specified.
- Reported versions: source-built uv 0.13.0, Rust 1.98.1, CPython 3.14.8 at `/usr/local/bin/python`, and ksh reporting `@(#)PD KSH v5.2.14 99/07/13.2`.
- Reported command: `uv venv`, followed by the printed `source .venv/bin/activate`.
- Expected behavior: a usable activation command for OpenBSD ksh; the reporter proposes `. .venv/bin/activate`.
- Actual reported behavior: environment creation succeeds, but ksh cannot find `source`.
- No project configuration, dependencies, or upgrade regression is reported. This concerns shell activation guidance, not dependency resolution or environment creation.

## Reproduction

**Outcome: reproducible**, with a platform qualification: runtime validation used portable OpenBSD ksh on Linux, not a native OpenBSD installation.

### Environment and isolation

- Host: Ubuntu 24.04.5, Linux x86_64.
- Executable: installed `uv` from PATH, `/opt/hostedtoolcache/uv/0.13.0/x86_64/uv`, reporting `uv 0.13.0 (x86_64-unknown-linux-gnu)`; no checkout build was used.
- Interpreter: CPython 3.12.3 at `/usr/bin/python3`, rather than the reporter's CPython 3.14.8.
- Shell: unmodified oksh 7.9, distributed as portable OpenBSD 7.9 ksh, locally compiled with `CFLAGS="-O0 -g" ./configure` and `make -j2`. Invoked through a temporary symlink named `ksh`, it reports `@(#)PD KSH v5.2.14 99/07/13.2` and `oksh 7.9`.
- Shell source: https://github.com/ibara/oksh/releases/download/oksh-7.9/oksh-7.9.tar.gz; SHA-256 `51b2d92515950c959dbf24f6fc33336db8c0526c2a50fee4ca598a18a6114a49`.
- Commands ran in an empty temporary project with a clean subprocess environment. `SHELL` pointed to the temporary `ksh`; no competing shell-version variables were inherited. PATH contained the temporary shell directory, the installed uv directory, `/usr/bin`, and `/bin`. Caches, temporary files, Python-install storage, and XDG directories were directed beneath the reproduction directory. Configuration loading and network use were disabled for environment creation.

### Commands and observed results

With `ksh` available as described above, run from the empty temporary project:

```sh
ksh -c 'uv venv --python /usr/bin/python3 --no-config --offline'
ksh -c 'source .venv/bin/activate'
ksh -c '. .venv/bin/activate && python -c "import sys; print(sys.prefix)"'
```

Environment creation exits 0 and prints:

```text
Using CPython 3.12.3 interpreter at: /usr/bin/python3
Creating virtual environment at: .venv
Activate with: source .venv/bin/activate
```

The printed activation command exits 127 and prints exactly:

```text
ksh: source: not found
```

The dot-command check exits 1 before Python executes:

```text
ksh: .venv/bin/activate[41]: ${.sh.file}": bad substitution
```

The generated activation script assigns `SCRIPT_PATH="${.sh.file}"` whenever `KSH_VERSION` is present, even for this ordinary, non-relocatable environment. An independent `ksh -c 'printf "%s\n" "${.sh.file}"'` also exits 1 with a bad substitution, confirming that this expansion is unsupported in the tested shell. This additional failure was observed locally; the report does not say whether it occurs on the reporter's native OpenBSD installation.

### Existing test coverage

The initial reproduction search of `crates/uv/tests/`, including its integration-test modules, and `crates/uv-client/tests/it/` found no ksh-specific activation test. The parent regression pull request, astral-sh/uv-dev#2622, subsequently added `create_venv_ksh`; this fix updates that test to require the dot-command hint. The relevant venv integration tests are in `crates/uv/tests/python/venv.rs`, whose module is gated by `test-python` in `crates/uv/tests/python/main.rs`.

- `create_venv` checks successful creation and snapshots the `source` hint. Its `TestContext` sets `SHELL=bash` on Unix in `crates/uv-test/src/lib.rs`; it does not execute the printed command or cover OpenBSD ksh.
- `create_venv_apostrophe` is Linux-only. It snapshots quoted activation guidance, then uses `. bin/activate` in the ambient shell and asserts the resulting Python prefix. It does not explicitly select ksh or execute the printed `source` command.
- `verify_pyvenv_cfg_relocatable` checks generated script text and relocatable configuration, without executing ksh.

These neighboring tests do not explicitly cover the reported ksh hint failure. The updated parent regression supplies that coverage. The initial reproduction used the installed executable; focused debug-profile checkout tests were subsequently run during the fix investigation described below.

### Retained artifacts and limits

The initial reproduction recorded artifacts at `/tmp/uv-22537.PyExvR` (a temporary location from that session): shell source and build logs, the generated `.venv` under `project/`, `reproduction-results.json`, `reproduction-environment.json`, and `script-path-check.json`.

The reported `source: not found` behavior was observed directly, rather than inferred from source. Native OpenBSD, the reporter's Python version, and their exact source-build configuration remain untested. Successful activation with the dot command must not be claimed from this reproduction.

## Classification

Bug: uv gives an unusable activation command for the tested OpenBSD-derived ksh, despite successful environment creation and an explicit ksh shell selection. The original implementation corroborated the runtime observation: `Shell::Ksh` shared the Bash/Zsh branch that emitted `source`. The fix separates ksh and emits `.`.

There is no established recent regression. Prior issue-context research found that astral-sh/uv#5640 introduced this branch in July 2024. The additional activation-script failure means a hint-only correction should not be presented as a complete OpenBSD compatibility fix without further validation.

## Related

- astral-sh/uv-dev#2622 — parent regression pull request. Added the passing `create_venv_ksh` test that snapshotted the incorrect hint; this fix retains it and changes its expected output to `.`.

- astral-sh/uv#5640 — fix(venv): make relocatable activation scripts support ksh (pull request, merged July 31, 2024). Introduced `Shell::Ksh` detection and added it to the Bash/Zsh branch that prints `source`. Its principal script change concerned locating a sourced script for relocatable environments; it is historical context, not a prior fix for OpenBSD ksh's missing `source` command. The locally observed `${.sh.file}` failure also makes its script-path logic relevant to follow-up validation.
- astral-sh/uv#18773 — `uv venv` recommends use of `source` in incompatible shells (issue, closed April 1, 2026). Prior research found that the reporter confirmed a login-shell versus current-shell mismatch: correcting `SHELL` removed the suggestion. Here, explicit ksh selection still produces `source`, so that confirmed mechanism differs.
- astral-sh/uv#2174 and astral-sh/uv#2221 — historical fish activation-hint issue and its fix, merged March 5, 2024. Background for shell-specific hints; the change did not yet include ksh.
- astral-sh/uv#7480 — yash rejects zsh-specific syntax inside the activation script. Different shell and failing expression; not a duplicate of the unavailable-command report.
- astral-sh/uv#10498 — unset `OSTYPE` with `set -u` on Alpine. Different script-level failure; not a duplicate of this report or the observed `${.sh.file}` failure.

## Supporting implementation evidence

At checkout `a79aac95dc8f1d79e4396fdcb262d22c520cc1f6`:

- `crates/uv/src/commands/venv.rs:390` selects `source` for `Shell::Bash | Shell::Zsh | Shell::Ksh`, using `shlex_posix` for the script path.
- `crates/uv-shell/src/lib.rs:66` recognizes exported `KSH_VERSION`; the shell-name parser also recognizes `ksh`, so correcting shell detection alone does not change this hint.
- `crates/uv-virtualenv/src/activator/activate:39` selects the `${.sh.file}` assignment when `KSH_VERSION` is present. The generated script and the isolated expansion check reproduce the additional incompatibility on oksh.

## Prior search coverage

The existing issue-context research searched open and closed issues and pull requests for ksh, OpenBSD activation, `KSH_VERSION`, `Shell::Ksh`, `source: not found`, Korn/POSIX/dot activation, and shell detection. It read candidate comments, issue timelines, source-file history, and the diffs of astral-sh/uv#5640 and astral-sh/uv#2221. No matching duplicate or prior fix for the specific ksh hint was found. PR keyword searches returned no entries despite historical PRs being retrievable directly, so absence of another tracker remains a search finding, not proof.

## Fix

**Outcome: fixed for the reported activation-command hint.**

In `crates/uv/src/commands/venv.rs`, `Shell::Ksh` now has a separate match arm that renders `. <activation-script>` using the same `shlex_posix` path quoting. Bash, Zsh, and the remaining shell branches are unchanged. This corrects the confirmed cause: selecting the unavailable `source` command for OpenBSD ksh.

The original parent regression, `create_venv_ksh` in `crates/uv/tests/python/venv.rs`, remains in place with its environment-creation assertion and now expects `Activate with: . .venv/bin/activate`. It retains its Unix gate and the module's `test-python` gate. The literal `bin` is appropriate for this Unix-only test: the shared snapshot filter normalizes `bin` only in hints beginning with `source`.

The parent test was first run unchanged and passed while asserting the incorrect hint. Its expectation was changed before production code, and it failed specifically because uv still emitted `source` instead of `.`. After the production fix and the Unix snapshot-path correction, all three focused tests passed with CPython 3.12.9 in the unoptimized debug/test profile:

```sh
INSTA_UPDATE=no cargo test -p uv --test python -- venv::create_venv_ksh venv::create_venv venv::create_venv_apostrophe --exact
RUSTUP_TOOLCHAIN=stable cargo fmt --all
git diff --check
```

`create_venv` verifies unchanged Bash hints and environment creation. `create_venv_apostrophe` verifies quoting and a generated activation-script/Python-prefix round trip in the ambient shell. Formatting used installed stable rustfmt because the pinned 1.99.0 toolchain lacks that component and its installation directory is read-only. Formatting and whitespace checks succeeded; only the two intended Rust files changed.

The investigation also inspected shell detection, path quoting, `prepend_path`, the activation template, and activation snapshots in the sync and pip-sync integration modules. Those snapshots select Bash and invoke the same venv producer, so they do not encode the incorrect ksh assumption. No distinct additional manifestation of the confirmed command-selection cause was found, and no redundant tests were added.

The `${.sh.file}` bad substitution observed during reproduction is a separate activation-template compatibility issue, not a result of choosing `source` instead of `.`. It is outside this focused fix and remains unresolved. Native OpenBSD and successful activation under OpenBSD ksh were not validated; the fixed hint must not be described as a complete activation-script compatibility fix.

No commits, pushes, Git configuration changes, or GitHub changes were made. Within the checkout, only the affected production file and the original parent regression-test file were modified.

Pull request: https://github.com/astral-sh/uv-dev/pull/2623
