# Generated files can follow existing destination links

Issue: astral-sh/uv#23056

Classification: bug

## Summary

The checkout now contains a focused fix and updated regression coverage. Generated activation
scripts, `.gitignore`, `pyvenv.cfg`, and Python 3.9 distutils patch files are atomically replaced,
leaving existing destination-link targets unchanged. All 13 focused debug integration tests pass.

Before the fix, the reported behavior was reproduced with the installed uv 0.13.6 on Ubuntu 24.04.5 Linux
x86_64 and CPython 3.12.3. Given an existing `v/bin/activate` symlink to a synthetic file
outside `v`, `uv venv --allow-existing v` succeeds, leaves the symlink in place, and replaces
the target's contents with the generated activation script. Separate executions also confirmed
external-target overwrites through `v/.gitignore` and `v/pyvenv.cfg` symlinks.

The report specifies Fedora Linux, uv 0.13.6, and any Python version. Fedora itself was not
tested. The fix makes generated-file replacement leave external symlink targets unchanged. The
reproduction and fix do not establish a security classification. The report's broader reference to
`_virtualenv.py` and `_virtualenv.pth` needs a version qualification: uv 0.13.6 generates
those files only below Python 3.10. That branch was initially only inspected; the fix investigation subsequently reproduced both
patch-file overwrites on CPython 3.9.25 and added coverage to the existing Python 3.9 test.

No confirmed duplicate. astral-sh/uv#23055 is the companion directory-symlink report;
astral-sh/uv#5152 and astral-sh/uv#5165 provide the closest file-write precedent.
astral-sh/uv#21857 and astral-sh/uv#21891 cover analogous truncation in Git checkout markers.

## Classification

This is a generated-file overwrite bug: direct create/truncate writes follow destination-file
symlinks and change external targets when existing entries are allowed. The fix replaces the
destination entries instead. This correctness finding does not establish that the scenario
qualifies as a vulnerability. No regression after an upgrade was alleged, and no
comparison with an older uv version was necessary to reproduce the stated example.

The distinction from astral-sh/uv#23055 matters: replacing a final file entry can prevent
changing that file's symlink target while still following a linked parent directory. These
are separate cases. Historical installer and Git fixes concern different subsystems and do
not establish either a duplicate or a regression here.

The reporter cites CVE-2026-113721 and pypa/virtualenv commit
`d25f28699eefb1fc34a251e5adf2b5de844c36cd`, and says Astral's security contact considered the
scenario outside uv's threat model. These are reporter-provided context, not independently
verified CVE applicability or a new security assessment.

## Reproduction

Outcome: **reproducible**.

### Initial reproduction environment and isolation

- Installed executable: `/opt/hostedtoolcache/uv/0.13.6/x86_64/uv` from `PATH`;
  `uv --version` reports `uv 0.13.6 (x86_64-unknown-linux-gnu)`.
- Host: Ubuntu 24.04.5 LTS, Linux `7.0.0-1017-azure`, x86_64.
- Interpreter: `/usr/bin/python3`, CPython 3.12.3.
- All fixtures, external sentinel files, temporary files, and caches were under
  `/tmp/uv-23056-g4_p3027`. The external targets were outside each virtual environment but
  inside this temporary reproduction directory.
- uv ran with a fresh environment, `--no-config`, `--offline`, an explicit interpreter, and
  Python downloads disabled. No project configuration or dependencies were needed.

### Minimal activation-file case

The following reconstructs the executed fixture and command with a fresh temporary path:

```sh
uv_executable=$(command -v uv)
repro_dir=$(mktemp -d /tmp/uv-23056-XXXXXX)
cd "$repro_dir"
mkdir -p v/bin tmp
printf 'SYNTHETIC_SENTINEL\n' > outside_file
ln -s "$PWD/outside_file" v/bin/activate
env -i PATH=/usr/bin:/bin \
  TMPDIR="$repro_dir/tmp" \
  UV_CACHE_DIR="$repro_dir/cache" \
  UV_PYTHON_INSTALL_DIR="$repro_dir/python" \
  XDG_CONFIG_HOME="$repro_dir/config" \
  XDG_DATA_HOME="$repro_dir/data" \
  XDG_CACHE_HOME="$repro_dir/xdg-cache" \
  UV_PYTHON_DOWNLOADS=never UV_NO_PROGRESS=1 \
  "$uv_executable" --no-config --offline venv \
  --python /usr/bin/python3 --allow-existing v
test -L v/bin/activate
head -n 1 outside_file
```

Observed exit code: **0**. stderr:

```text
Using CPython 3.12.3 interpreter at: /usr/bin/python3
Creating virtual environment at: v
```

The outside file changed from `SYNTHETIC_SENTINEL\n` to a 4,237-byte activation script in
the executed fixture. Its first line was
`# Copyright (c) 2020-202x The virtualenv developers`; its contents included the
`VIRTUAL_ENV=` assignment and no longer contained the sentinel. The destination remained
a symlink and the target's inode was unchanged. Script size depends on the temporary path.

### Additional observed cases

Each case used a separate directory, a new outside file containing the same sentinel, the
same isolated environment, and the same uv/Python versions. These were explicit independent
executions, not changes to repository tests.

| Destination | Invocation after fixture setup | Observed result |
| --- | --- | --- |
| `v/.gitignore` symlink to `outside_file` | `uv --no-config --offline venv --python /usr/bin/python3 --allow-existing v` | Exit 0; outside file became exactly `*` with no newline; symlink remained. |
| `v/pyvenv.cfg` symlink to `outside_file` | `uv --no-config --offline venv --python /usr/bin/python3 --allow-existing v` | Exit 0; outside file became the generated configuration shown below; symlink remained. |
| `v/bin/activate` symlink, without permission to reuse the directory | `uv --no-config --offline venv --python /usr/bin/python3 v` | Exit 2, `A directory already exists at: v`; outside sentinel stayed unchanged and symlink remained. |

For the `.gitignore` and `pyvenv.cfg` cases, setup consisted of `mkdir v`, writing the
synthetic sentinel to `outside_file`, and creating the corresponding symlink with
`ln -s "$PWD/outside_file" v/.gitignore` or
`ln -s "$PWD/outside_file" v/pyvenv.cfg`. The control used the activation-file setup above.

The external `pyvenv.cfg` target became exactly:

```text
home = /usr/bin
implementation = CPython
uv = 0.13.6
version_info = 3.12.3
include-system-site-packages = false
```

Neither `_virtualenv.py` nor `_virtualenv.pth` was generated in the initial Python 3.12
environment, consistent with the source's below-3.10 condition. In the fix investigation,
CPython 3.9.25 was installed under `/tmp/uv-23056-fix-python`. A fixture under
`/tmp/uv-23056-patch-bcvt3q8q` placed links at `v/lib/python3.9/site-packages/_virtualenv.py`
and `_virtualenv.pth`, each pointing to a separate outside sentinel. Installed uv 0.13.6 ran
`uv --no-config --offline venv --allow-existing --python
/tmp/uv-23056-fix-python/cpython-3.9.25-linux-x86_64-gnu/bin/python3.9 v` and exited 0. Both
links remained; the outside module became 5,246 bytes of generated code and the outside `.pth`
target became exactly `import _virtualenv`. Thus both additional write sites were observed
to have the same cause. Other activation-script names, hardlinks, linked parent directories,
and Windows were not exercised as overwrite cases.

Initial reproduction recorded evidence files: `/tmp/uv-23056-g4_p3027/activate-result.json` and
`/tmp/uv-23056-g4_p3027/additional-results.json`.

### Integration-test coverage

The relevant setup and assertions in `crates/uv/tests/python/venv.rs` were inspected:

- `non_empty_dir_exists_allow_existing` creates a nonempty directory with a regular file,
  checks rejection without `--allow-existing`, and checks successful creation and repeated
  creation with that flag. It does not pre-plant a generated-file symlink or inspect an
  external target.
- `create_venv_then_allow_existing` checks successful creation followed by successful
  recreation with `--allow-existing`; it has no external-target assertion.
- `create_venv_symlink_clear_preservation` and `create_venv_nested_symlink_preservation`
  check that environment-root symlinks survive creation and `--clear`. They cover a
  different link position from a generated destination file.
- `create_venv_skips_distutils_patch_on_py310` asserts the patch files are absent on Python
  3.10; `create_venv_keeps_distutils_patch_on_py39` asserts they are files on Python 3.9.
  The Python 3.9 test now also checks regeneration through destination links and startup
  loading via the `.pth` file.
  `create_venv_preserves_existing_distutils_patch` places regular patch files in a Python
  3.10 environment, checks their continued existence with `--allow-existing`, and their
  absence after `--clear`; its ordinary-file preservation assertions remain appropriate.

The initial reproduction search found no coverage for this exact external-target overwrite.
The checkout now includes the three regression tests from astral-sh/uv-dev#3063:
`allow_existing_activate_symlink`, `allow_existing_gitignore_symlink`, and
`allow_existing_pyvenv_cfg_symlink`. All three were retained and updated to require complete
sentinel preservation, replacement of destination links, and valid generated contents or
consumer behavior. The activation test retains rejection without `--allow-existing` and
now sources `.venv/bin/activate`; the configuration test starts the environment's Python.

Related fixtures in `crates/uv/tests/pip_install/pip_install.rs` were also inspected, including
`launcher_with_symlink`, `install_relocatable`, and `install_script_with_symlinked_lib`.
These test execution through links, relocation, or linked parent directories, not generated
destination-file replacement. Their assumptions do not contradict this fix. No additional
command coverage was added for callers sharing the same virtualenv writer.

## Related

- astral-sh/uv-dev#3063 — Parent regression pull request. Its three tests initially passed while asserting external-target overwrites; all are updated in this fix.
- astral-sh/uv#23055 — Builtin creation follows linked environment directories (open issue). Companion report for uv 0.13.6 and `uv venv --allow-existing`: a pre-existing directory symlink redirects generated subtrees outside the environment. This report instead concerns individual destination-file symlinks; replacing those files does not address traversal through linked parent directories.
- astral-sh/uv#5152 — Uv script install into .venv overwrites python (closed issue). Confirmed historical example of a generated script following an existing file symlink and overwriting its external target. It involved installing a console entry point named python during `uv sync`, rather than generating virtual-environment files, and was fixed by astral-sh/uv#5165.
- astral-sh/uv#5165 — Make entrypoint writes atomic to avoid overwriting symlinks (merged pull request). Merged July 17, 2024; replaced direct entry-point writes with atomic writes to fix astral-sh/uv#5152. Its changes were confined to the wheel installer, providing a relevant implementation precedent without fixing the uv-virtualenv writes reported here.
- astral-sh/uv#21857 — uv Git checkout marker follows a repository-controlled symlink (closed issue). Another confirmed external-file truncation through an existing destination symlink, but triggered by writing a Git checkout's .ok marker during dependency installation. Its canonical fix is astral-sh/uv#21891; it does not cover virtual-environment generation.
- astral-sh/uv#21891 — Store Git checkout markers outside working trees (merged pull request). Merged September 23, 2026; fixes astral-sh/uv#21857 by moving checkout markers outside repository-controlled working trees. Maintainer comments on the closed alternative astral-sh/uv#21892 explicitly select this fix. It changes the Git/cache subsystem, not destination-file writes in uv-virtualenv.

## Supporting implementation evidence

The retained issue context identifies checkout commit `39a02a0` and uv version `0.13.6`;
the relevant implementation was read during reproduction. The following locations describe
the pre-fix implementation and line numbers:

- `crates/uv-virtualenv/src/virtualenv.rs:145`: `OnExisting::Allow` permits an existing
  directory without clearing its entries.
- `crates/uv-virtualenv/src/virtualenv.rs:215`: `.gitignore` uses `fs_err::write`.
- `crates/uv-virtualenv/src/virtualenv.rs:539`: activation templates use `fs_err::write`.
- `crates/uv-virtualenv/src/virtualenv.rs:596`: `pyvenv.cfg` uses `File::create`.
- `crates/uv-virtualenv/src/virtualenv.rs:621`: the distutils patch files use
  `fs_err::write`; the condition at line 50 limits their creation to Python below 3.10.

The observed writes through the activation, `.gitignore`, and `pyvenv.cfg` links agree with
these direct create/truncate call sites. The reproduction result is based on executed
commands, not solely on source inspection.

Prior triage identified atomic generated-file writes in
`crates/uv-install-wheel/src/wheel.rs:912` and the temporary-file/rename helper in
`crates/uv-fs/src/lib.rs:558`, consistent with astral-sh/uv#5165. The related changes in
astral-sh/uv#5165 and astral-sh/uv#21891 do not cover `uv-virtualenv`.

## Fix

Outcome: **fixed** in the checkout; no commit, push, or GitHub modification was performed.

### Implementation

`crates/uv-virtualenv/src/virtualenv.rs` now uses the existing `uv_fs::write_atomic_sync`
helper for activation templates, `.gitignore`, `pyvenv.cfg`, `_virtualenv.py`, and
`_virtualenv.pth`. Configuration bytes are assembled in memory before replacement. The
helper writes a temporary file in the destination directory and renames it over the final
entry, so an existing destination symlink's target is not opened for writing. No shared
filesystem helper, dependency, or unrelated command implementation was changed.

The parent regression file `crates/uv/tests/python/venv.rs` is updated, not replaced or
weakened. The existing `create_venv_keeps_distutils_patch_on_py39` test is extended with
explicit patch-link fixtures and a Python startup check. Its `test-python-eol` gate and the
module's `test-python` gate remain in place; symlink setup is Unix-only. There are no new
test functions or repeated option matrices.

`CACHEDIR.TAG` already uses create-new semantics; interpreter links already use replacement
helpers. Installer-generated files use a separate atomic writer. None required changes.
Directory-link traversal remains a separate concern tracked by astral-sh/uv#23055; this
fix does not attempt to change that behavior.

### Validation

Before production changes, `cargo test -p uv --test python allow_existing_ -- --nocapture`
passed all three original parent regressions. After their assertions were corrected and the
Python 3.9 test extended, `cargo test -p uv --test python -- allow_existing_
venv::create_venv_keeps_distutils_patch_on_py39` failed all four tests specifically because
external sentinels had been replaced with generated content. The failure log is
`/tmp/uv-23056-red.log`.

After the fix, this debug-profile command passed all 13 selected tests:

```sh
cargo test -p uv --test python -- --exact \
  venv::allow_existing_activate_symlink \
  venv::allow_existing_gitignore_symlink \
  venv::allow_existing_pyvenv_cfg_symlink \
  venv::create_venv_keeps_distutils_patch_on_py39 \
  venv::create_venv_skips_distutils_patch_on_py310 \
  venv::create_venv_preserves_existing_distutils_patch \
  venv::create_venv \
  venv::non_empty_dir_exists_allow_existing \
  venv::create_venv_then_allow_existing \
  venv::create_venv_symlink_clear_preservation \
  venv::create_venv_nested_symlink_preservation \
  venv::verify_pyvenv_cfg \
  venv::verify_pyvenv_cfg_relocatable
```

The successful checks include shell activation followed by Python execution, Python consuming
`pyvenv.cfg`, and Python loading `_virtualenv` through the generated `.pth` file. They also
cover ordinary creation/reuse, environment-root symlink preservation, Python 3.10 patch
omission/preservation, and relocatable configuration. The log is `/tmp/uv-23056-green.log`.

`RUSTUP_TOOLCHAIN=stable cargo fmt --all` and `git diff --check` succeeded. The pinned
1.99.0 toolchain lacks rustfmt and its installation directory is read-only, so formatting
used the already installed stable rustfmt 1.10.0. Only the two intended Rust files are
modified. No full test suite, release build, or Windows validation was run.

## Search scope and exclusions

The following records the earlier related-issue triage; GitHub searches were not repeated
for this reproduction pass.

The report was decomposed before searching into the observable external-file overwrite;
the `uv venv --allow-existing` trigger; each generated-file family; the existing
destination-file symlink condition; and the exact CVE identifier. Searches for the
observable behavior were kept separate from searches for atomic-write or link-handling
mechanisms. Broader searches omitted Fedora, the uv version, and incidental file names.

Used authenticated gh to search astral-sh/uv issues across open and closed states and pull requests without a state restriction, with additional explicit open, closed, and merged PR attempts. Literal searches covered CVE-2026-113721, destination links, --allow-existing with symlink, activate, .gitignore, and _virtualenv.py. Conceptual and fix-oriented searches covered symbolic links, overwrite, truncate, clobber, hardlinks, atomic writes, generated files, paths outside the environment, and area:environments. Inspected candidate bodies, comments, reviews, timelines, and changed-file lists; followed astral-sh/uv#5152 to astral-sh/uv#5165 and astral-sh/uv#21857 through astral-sh/uv#21892 to astral-sh/uv#21891. Ruled out astral-sh/uv#14670 and astral-sh/uv#14933 as duplicates: they preserve the environment-root symlink during recreation, not individual generated-file targets. PR search results were incomplete despite directly retrievable historical PRs, and later REST searches hit an installation rate limit; direct GraphQL PR inspection remained available. No exact duplicate or prior fix for this virtualenv-file path was confirmed.

astral-sh/uv#14670 and its merged fix astral-sh/uv#14933 were particularly plausible from
their titles, but their desired behavior is to retain and follow an environment-root
symlink while recreating its target. Maintainer comments confirm that intent. They do not
track writes through symlinks at generated file destinations. The broader
astral-sh/uv#1472 discussion concerns clearing an existing environment, not this final-file
write behavior.

Search limitations mean that the absence of an exact duplicate is not an exhaustive claim.
The strongest relationships above were verified through direct issue and pull request views.

Pull request: https://github.com/astral-sh/uv-dev/pull/3066
