# Builtin creation follows linked environment directories

Issue: astral-sh/uv#23055

Classification: bug

## Summary

The behavior in astral-sh/uv#23055 is **reproducible** with the installed uv 0.13.6 on Ubuntu 24.04.5 LTS, Linux x86_64, using CPython 3.12.3. With `v/bin` linked to a sibling directory, `uv venv --allow-existing v` exits successfully and creates activation scripts and interpreter links in that sibling directory. A separate `v/lib` fixture also redirects creation of `python3.12/site-packages` outside the environment. Both child symlinks remain in place.

The reporter used Fedora Linux and uv 0.13.6, with Python specified as “any”; no project configuration or dependencies are required. Fedora itself was not tested. The report implies that linked child directories should not redirect creation outside the environment, but does not choose between rejecting and replacing such links. Reproduction confirms the filesystem behavior, not whether uv should change its compatibility policy.

No exact duplicate was identified in the prior issue-context investigation. astral-sh/uv#23056 is the closest companion report, covering file symlinks. Other related work concerns console-script installation, intentional environment-root symlinks, and wheel-write compatibility.

The reporter associates the behavior with CVE-2026-113716 in pypa/virtualenv and says Astral's security contact considered it outside uv's attacker and threat models. That correspondence is reporter-provided context, not independently verified evidence of uv vulnerability status.

## Classification

Retain the reported **bug** classification for maintainer triage: the observed behavior matches the report. Whether linked child directories should be rejected, replaced, or supported remains a compatibility decision. The documented meaning of `--allow-existing` explains why existing contents are retained, but does not settle that policy.

astral-sh/uv#23056 describes symlinks at individual output-file paths. A file-level atomic replacement can address that scenario while still following a symlink in a parent directory. The two reports therefore need distinct coverage and are not treated as duplicates. No regression after an upgrade or last known-good version was reported or established.

## Reproduction

**Outcome: reproducible.** Three explicit command invocations tested linked `bin`, linked `lib`, and a control without `--allow-existing`.

### Environment and isolation

- Installed executable from `PATH`: `/opt/hostedtoolcache/uv/0.13.6/x86_64/uv`, reporting `uv 0.13.6 (x86_64-unknown-linux-gnu)`.
- Host: Ubuntu 24.04.5 LTS; Linux `7.0.0-1017-azure`, x86_64.
- Interpreter: `/usr/bin/python3.12`, CPython 3.12.3.
- No `pyproject.toml`, dependencies, seed packages, or preview features. Commands used `--no-config --offline` and an explicit interpreter; `UV_PYTHON_DOWNLOADS=never` prevented downloads.
- All fixtures, command logs, uv/Python directories, XDG directories, and temporary files were confined to `/tmp/uv-23055-aeptefi4`. Subprocesses received an explicitly constructed environment instead of inheriting user configuration or credentials.

### Minimal fixture and commands

The following shell commands reconstruct the tested layouts. The actual invocations ran from each scenario directory with destination `v`.

```sh
repro_dir=$(mktemp -d /tmp/uv-23055-XXXXXX)
export UV_CACHE_DIR="$repro_dir/cache"
export UV_PYTHON_INSTALL_DIR="$repro_dir/python"
export UV_PYTHON_DOWNLOADS=never

mkdir -p "$repro_dir/linked-bin/v" "$repro_dir/linked-bin/outside_pkgs"
cd "$repro_dir/linked-bin"
ln -s "$PWD/outside_pkgs" v/bin
uv --no-config --offline venv --python /usr/bin/python3.12 --allow-existing v
test -L v/bin
test -f outside_pkgs/activate
```

Observed exit status: **0**. Both filesystem checks succeed. `outside_pkgs` contains `activate`, `activate.bat`, `activate.csh`, `activate.fish`, `activate.nu`, `activate.ps1`, `activate.xsh`, `activate_this.py`, `deactivate.bat`, `pydoc.bat`, `python`, `python3`, and `python3.12`. Thus the executable and activation subtree lands outside `v` while `v/bin` remains a symlink. Standard output is empty; standard error is:

```text
Using CPython 3.12.3 interpreter at: /usr/bin/python3.12
Creating virtual environment at: v
```

A separate library-directory fixture:

```sh
mkdir -p "$repro_dir/linked-lib/v" "$repro_dir/linked-lib/outside_lib"
cd "$repro_dir/linked-lib"
ln -s "$PWD/outside_lib" v/lib
uv --no-config --offline venv --python /usr/bin/python3.12 --allow-existing v
test -L v/lib
test -d outside_lib/python3.12/site-packages
```

Observed exit status: **0**, with the same standard error. Both checks succeed. `outside_lib/python3.12/site-packages` is created outside `v`. It is empty for this unseeded Python 3.12 environment; this scenario demonstrates redirected directory creation, not package-file writes.

Control using a fresh linked `bin` directory, omitting `--allow-existing`:

```sh
mkdir -p "$repro_dir/no-allow-existing/v" "$repro_dir/no-allow-existing/outside_pkgs"
cd "$repro_dir/no-allow-existing"
ln -s "$PWD/outside_pkgs" v/bin
uv --no-config --offline venv --python /usr/bin/python3.12 v
```

Observed exit status: **2**, with `error: Failed to create virtual environment` and `cause: A directory already exists at: v`. The external `outside_pkgs` directory remains empty.

Raw output is retained in each scenario's `stdout.txt` and `stderr.txt` under `/tmp/uv-23055-aeptefi4`; `results.json` records the executable, interpreter, exit statuses, and positive filesystem checks.

### Existing test coverage

The relevant integration module is `crates/uv/tests/python/venv.rs`. Its following test setups and assertions were read:

- `non_empty_dir_exists_allow_existing`: starts with an ordinary file in the environment directory, snapshots failure without `--allow-existing`, then two successful creations with it. It does not pre-plant a directory symlink.
- `create_venv_then_allow_existing`: snapshots successful creation and a successful second invocation with `--allow-existing`; no linked child directory is set up.
- `create_venv_symlink_clear_preservation` and `create_venv_nested_symlink_preservation`: create single or chained symlinks at the environment root, snapshot creation and `--clear`, and assert the root links remain. These Unix-only tests cover supported root links, not linked child directories. The module is gated by `test-python` in `crates/uv/tests/python/main.rs`.

The broader search of `crates/uv/tests/` and `crates/uv-client/tests/it/` also found `crates/uv/tests/pip_install/pip_install.rs::install_script_with_symlinked_lib`. It moves an already-created environment's `lib` to `usr/lib`, links `lib` to it, installs a local fixture, snapshots exit status 2, and asserts that the launcher is misplaced in `usr/bin` instead of `bin`. This covers the separate installer behavior in astral-sh/uv#21255, not builtin environment creation or writes outside the environment root.

The parent regression PR, astral-sh/uv-dev#3062, subsequently added `create_venv_allow_existing_symlink_bin` and `create_venv_allow_existing_symlink_lib` in this module. Both are Unix-only tests under the module's `test-python` gate. They snapshot successful creation and explicitly assert the undesirable external writes: an activation script and interpreter symlink for `bin`, and the `python3.12/site-packages` subtree for `lib`. Both passed during the fix investigation on the unchanged parent checkout, as detailed below.

### Limits and compatibility

Fedora, Windows junctions, other Python versions, and a symlink directly at the final `site-packages` component were not separately tested. No older uv comparison was needed to confirm the reported version, and no regression boundary is established. The result does not independently establish a security vulnerability. Any change to child-directory handling needs a compatibility decision separate from supported symlinks at the environment root.

## Fix

**Outcome: not fixed; checkout unchanged.** A compatibility decision is required before selecting the desired assertions and production behavior. The report and parent regression PR establish that writes escape the environment, but do not specify whether `--allow-existing` should reject the affected directory layout or replace its child links. The documented option promises to write into existing contents without clearing them, and `OnExisting::Allow` explicitly retains other files. Replacing child directory links changes that retention behavior; rejecting escaping links introduces a new restriction on existing layouts. No such policy is established by the inspected report, parent PR, or implementation.

### Confirmed cause and scope

The reproduction and passing parent regressions agree with `crates/uv-virtualenv/src/virtualenv.rs`: `OnExisting::Allow` retains child directory symlinks, `create_dir_all(&scripts)` follows a linked executable directory, and subsequent interpreter-link and activation-file writes reach its target. Separately, `create_dir_all(&site_packages)` follows the linked library directory and creates its subtree outside the environment. These are the two concrete manifestations already covered by the parent regressions; no additional speculative scenarios were added.

Neighboring creation tests confirm that nonempty-directory reuse and repeated `--allow-existing` invocations are supported. The root-symlink preservation tests explicitly require following and retaining single or chained environment-root links, including when clearing the environment; both still pass. The creator also intentionally generates `lib64 -> lib` on applicable Unix systems. A future containment rule must distinguish these supported links from whichever child links the chosen policy disallows.

The creator's returned scheme is passed through `Interpreter::with_virtualenv`, and the command's optional seeding consumes the created environment. Neither provides a separate containment policy. The inspected `install_script_with_symlinked_lib` fixture in `crates/uv/tests/pip_install/pip_install.rs` redirects a launcher within an already-created environment during wheel installation. That separate relative-path problem does not establish another manifestation requiring a change to builtin creation, so its fixture and assertions were left intact. Individual output-file symlinks remain the separate companion report.

### Focused validation

The unchanged parent regression tests were first executed successfully in the debug test profile:

```sh
cargo test -p uv --test python --no-default-features --features test-python create_venv_allow_existing_symlink -- --nocapture
```

Result: **2 passed**, using CPython 3.12.16. Both snapshots report exit status 0 and their external-write assertions pass. This confirms the tests reproduce the reported behavior before any prospective fix.

That reduced feature selection emitted an unused-import warning in `python_pin.rs`: its `EnvVars` use is gated by `test-python-managed` while its import is unconditional. The warning was traced to that feature combination, without modifying the unrelated file. A second focused run enabled the corresponding feature and included the supported root-link cases:

```sh
cargo test -p uv --test python --no-default-features --features test-python,test-python-managed -- --exact venv::create_venv_allow_existing_symlink_bin venv::create_venv_allow_existing_symlink_lib venv::create_venv_symlink_clear_preservation venv::create_venv_nested_symlink_preservation
```

Result: **4 passed**, 191 filtered out, with no compiler warning. These are baseline and compatibility checks, not validation of a fix. No desired-behavior snapshot was substituted: doing so would encode the unresolved reject-versus-replace policy. No production or test files were changed, and no formatting run was necessary. `git diff --exit-code` and `git status --short` confirmed the checkout remained clean.

## Related

- astral-sh/uv-dev#3062 — Parent regression PR. Adds the linked `bin` and `lib` creation tests; the unchanged checkout contains both and both pass while asserting the reported behavior.

- astral-sh/uv#23056 — Generated files can follow existing destination links (issue, open). Companion report for uv 0.13.6 and the same `uv venv --allow-existing` command: generated files overwrite targets of pre-existing file symlinks. This report instead follows directory symlinks such as `v/bin`; replacing individual output files atomically would not prevent that parent-directory traversal.
- astral-sh/uv#21255 — uv sync and uv pip install fails for packages with console scripts when the venv contains a `lib` to `usr/lib` symlink (issue, open). Also demonstrates writes redirected by a linked directory inside a virtual environment. It concerns console-script paths during package installation and a subsequent metadata error, whereas this report concerns environment creation itself and writes outside the environment.
- astral-sh/uv#21256 — normalize paths to avoid traversing symlinks when writing console scripts (pull request, open). Proposes a fix for astral-sh/uv#21255 in wheel installation and uninstallation. Review discusses relative paths through linked library directories. It does not change the recursive directory creation or generated-file writes in uv-virtualenv.
- astral-sh/uv#14933 — Fix symlink preservation in virtual environment creation (pull request, merged). Fixed astral-sh/uv#14670 by preserving a symlink at the environment root when clearing or recreating it. Maintainer comments explicitly support following that root symlink. It did not validate linked child directories, so this report is not a regression of that fix.
- astral-sh/uv#21997 — Fix wheel file replacement and stale data handling (pull request, open). Related filesystem-write work in the wheel installer. Its current description explicitly retains destination directory symlinks, and maintainer discussion explains compatibility with distribution-managed layouts. It follows the reverted guard in astral-sh/uv#21569 and concerns a different subsystem from builtin environment creation.

## Supporting evidence

### Current source

The checkout identifies both the uv executable crate and uv-version as 0.13.6.

- `crates/uv-virtualenv/src/virtualenv.rs:110`: the initial metadata check validates the environment location. In the `OnExisting::Allow` branch, existing contents are retained without validating child-directory components.
- `crates/uv-virtualenv/src/virtualenv.rs:252`: the scripts directory is created with `fs_err::create_dir_all(&scripts)`. Unix interpreter links are then written beneath that directory.
- `crates/uv-virtualenv/src/virtualenv.rs:539`: activation templates are written with `fs_err::write(scripts.join(name), activator)`. A symlink in the scripts-directory path redirects these writes.
- `crates/uv-virtualenv/src/virtualenv.rs:600`: the purelib/site-packages path is joined to the environment location and passed to `create_dir_all` at line 602. This likewise does not reject linked directory components. Distutils compatibility files are written there for Python versions below 3.10.
- `crates/uv-cli/src/lib.rs:2971`: `--allow-existing` is documented as writing into the existing location without clearing it.
- `crates/uv-fs/src/lib.rs:1086`: `clear_virtualenv` canonicalizes the environment root before clearing it, consistent with the supported root-symlink behavior discussed in astral-sh/uv#14933.

These source paths are consistent with the observed redirection in the reproduction above. The installed uv executable, rather than source inspection alone, establishes reproducibility. Windows junction behavior was not verified.

### Historical changes and compatibility

astral-sh/uv#14670 described replacement of a symlink at the environment root on repeated creation. Its merged fix, astral-sh/uv#14933, changed clearing/recreation of that root. It did not check child directories and does not establish a regression here.

astral-sh/uv#21569 introduced validation of wheel-installation destinations in September 2026. It was reverted by merged astral-sh/uv#21699 after installation regressions, including astral-sh/uv#21692; a maintainer confirmed that regression was fixed in uv 0.12.15. Closed, unmerged astral-sh/uv#21698 led to open astral-sh/uv#21997. The latter's current description explicitly retains destination directory symlinks, and discussion identifies distribution-managed layouts that legitimately cross installation prefixes. These changes concern wheel installation, not the builtin creator; their history does not establish an earlier fix for this report.

The reporter's upstream reference was inspected directly:
https://github.com/pypa/virtualenv/commit/554d9f9d4f7758f5cf53f993031e29787f1f74c5.
That commit replaces ordinary directory creation with `ensure_within` for executable/library paths and adds tests for existing symlinks and Windows junctions. It supports the technical analogy, but uv's compatibility policy must be considered independently.

## Search scope and exclusions

Searched astral-sh/uv open and closed issues and attempted open, closed, and merged PR searches with authenticated gh. Literal queries included CVE-2026-113716, the upstream commit hash, the title, --allow-existing, pre-planted links, and create_dir_all. Conceptual queries covered venv/virtualenv creation, directory and symbolic links, redirected/outside writes, activation files, junctions, and area:environments/security labels, without Fedora or version restrictions. PR keyword searches returned no candidates even for directly retrievable relevant PRs; supplemented them with paginated open-PR listings, a recent closed-PR listing, and direct inspection of referenced PRs, comments, reviews, and patches. Followed astral-sh/uv#14670 to astral-sh/uv#14933 and astral-sh/uv#21692 through astral-sh/uv#21569, astral-sh/uv#21699, astral-sh/uv#21698, and astral-sh/uv#21997. Ruled out astral-sh/uv#14249 and astral-sh/uv#17963: their fixed --allow-existing regressions concern Windows executable-link creation failing with os error 80. Also ruled out astral-sh/uv#15800 (cross-platform directory aliases) and astral-sh/uv#20431 (build-backend source containment). No inspected item tracks the same child-directory traversal during builtin creation.

Additional inspected context included astral-sh/uv#1863, whose maintainer comment confirms that creating an environment in a non-empty directory is supported via `--allow-existing`, and astral-sh/uv#1472, which discusses clearing existing environments by default. Neither tracks child-directory traversal.

## Verification status

The reproduction ran three isolated invocations of the installed uv 0.13.6, confirming linked executable-directory writes, linked library-directory creation, and the rejection control without `--allow-existing`. The source and test review support the interpretation; related GitHub context and search history above are retained from the prior investigation and were not re-queried during reproduction.

The initial reproduction used the installed binary; the fix investigation subsequently built the checkout in the debug test profile and ran only the focused tests listed in the Fix section. No full suite or release build was run. No checkout source or test files, Git configuration, or GitHub objects were changed. The fix investigation updated only this handoff README and generated local build/test artifacts; the other issue-context files were left unchanged.
