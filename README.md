# Generated POSIX console launchers interpret destination paths as shell syntax

Issue: astral-sh/uv#23057

Classification: bug

## Summary

The checkout now contains a focused fix and updated regressions for both console entrypoints and rewritten wheel data scripts. Tabs select the existing POSIX shell wrapper; both launch paths pass focused debug-profile integration tests.

The reported console-entrypoint launch failure is reproducible with the installed uv 0.13.6 on Ubuntu 24.04.5 LTS, Linux x86_64, and CPython 3.12.3. Creating a virtual environment under a short path containing a literal tab and installing a local wheel both succeed, but executing its generated console launcher fails with ENOENT. Bash reports `cannot execute: required file not found` and exits 127. The report describes the same failure on Fedora Linux with different shell diagnostic wording.

The affected environment is non-relocatable, its interpreter path contains a tab but no space, and its complete shebang line is at most 127 bytes. The reproduced shebang was 48 bytes. The same launcher runs successfully when passed explicitly to the environment's Python. A plain-path environment works, and an environment whose path contains both a space and a tab receives a working `/bin/sh` wrapper.

The title suggests shell interpretation, while the supplied reproduction demonstrates a direct-shebang interpreter lookup failure. This reproduction does not establish arbitrary shell execution or independently assess the reporter's cited CVE or security correspondence.

## Classification

Confirmed launcher-generation bug. A console entrypoint installed successfully into a valid environment should execute using that environment's Python. The observed failure is isolated to the generated direct shebang; the Python interpreter, installed module, and entrypoint body work.

Before the fix, the implementation selected the POSIX shell wrapper for spaces, long shebangs, or relocatable environments, but omitted tabs. The observed generated files and successful space-plus-tab control agree with this condition. The report does not identify a last known-good uv version or claim a regression after an upgrade. Historical astral-sh/uv#2097 addressed spaces and long paths; its omission of tabs does not establish that tab paths previously worked.

## Reproduction

**Outcome: reproducible before the fix.** No additional project configuration is needed for the reported case. The following observations describe the original installed uv 0.13.6 reproduction; checkout validation is recorded under Fix.

### Environment and isolation

- Installed executable: `/opt/hostedtoolcache/uv/0.13.6/x86_64/uv`, discovered on `PATH`; version output: `uv 0.13.6 (x86_64-unknown-linux-gnu)`.
- Host: Ubuntu 24.04.5 LTS; Linux `7.0.0-1017-azure`, x86_64. Fedora was not tested.
- Interpreter: `/usr/bin/python3`, CPython 3.12.3. Bash: 5.2.21.
- All fixtures, environments, and caches are under `/tmp/uv23057-aW1el1z4`.
- The subprocess environment was explicitly constructed with `UV_OFFLINE=true`, `UV_NO_CONFIG=true`, `UV_PYTHON_DOWNLOADS=never`, and temporary `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR`, `TMPDIR`, and XDG directories. No ambient authentication or Python environment settings were inherited. No network access or package build was needed.
- The environment used ordinary `uv venv`, without relocation. No project, workspace, dependency groups, or lockfile participated.

### Fixture and commands

The fixture is a locally generated, dependency-free `tab_probe-1.0-py3-none-any.whl`, created with Python's standard-library `zipfile`. It includes valid `METADATA`, `WHEEL`, and hashed `RECORD` files, plus:

```ini
# tab_probe-1.0.dist-info/entry_points.txt
[console_scripts]
tab-probe = tab_probe:main
```

```python
# tab_probe.py
import sys

def main():
    print("entrypoint-ok")
    print(repr(sys.executable))
    return 0
```

The original reproduction wrote its fixture generator, wheel, launchers, and results under `/tmp/uv23057-aW1el1z4`. Those temporary artifacts are not present in the fix environment. The commands and observations below preserve the findings; the two checked-in integration tests provide executable fixtures for the fix.

With that wheel and the isolated environment described above, the minimal commands are equivalent to the following Bash invocations from the temporary directory. `$'\t'` denotes a literal tab; every path is passed as one argument.

```bash
uv venv --python /usr/bin/python3 $'tab\tdir/venv'
uv pip install --python $'tab\tdir/venv/bin/python' ./tab_probe-1.0-py3-none-any.whl
$'tab\tdir/venv/bin/tab-probe'
```

Expected: the final command exits 0 and prints `entrypoint-ok` followed by the environment's Python path.

Observed:

1. Environment creation and wheel installation both exit 0.
2. The generated executable file starts with the following bytes, shown using Python byte-string notation: `b'#!/tmp/uv23057-aW1el1z4/tab\tdir/venv/bin/python\n'`. The full line is 48 bytes, contains a tab, and has no space. There is no `/bin/sh` wrapper.
3. Direct execution through `subprocess.run([launcher])` raises `FileNotFoundError`, errno 2, even though the launcher exists and is executable.
4. Executing the same absolute launcher path using `bash --noprofile --norc -c 'exec "$1"' bash "$launcher"` exits 127 with `cannot execute: required file not found`. This is equivalent to the report's `bad interpreter: No such file or directory` failure, with Bash-version-dependent wording.

### Controls

Explicitly invoking the interpreter bypasses the shebang and succeeds:

```bash
$'tab\tdir/venv/bin/python' $'tab\tdir/venv/bin/tab-probe'
```

It exits 0 and prints:

```text
entrypoint-ok
'/tmp/uv23057-aW1el1z4/tab\tdir/venv/bin/python'
```

Two separately created environments installed the identical wheel:

| Destination | Generated launcher | Execution |
| --- | --- | --- |
| `plain/venv` | Direct Python shebang | Exit 0; prints `entrypoint-ok` and the intended environment interpreter |
| `space tab\tdir/venv` (literal tab) | `/bin/sh` wrapper with a single-quoted interpreter path | Exit 0; prints `entrypoint-ok` and the intended environment interpreter |

The space-plus-tab control is relevant because the space independently selects the shell wrapper and masks the missing tab condition. Long paths and relocatable environments also select that branch in source, but those variants were not executed.

### Existing test coverage

The initial investigation found no short, tab-only console-entrypoint regression case in the inspected tests. Searches covered `crates/uv/tests/` and `crates/uv-client/tests/it/`. Subsequently, astral-sh/uv-dev#3064 added two integration tests asserting the undesirable behavior. Both are retained and updated by this fix.

- `crates/uv/tests/pip_install/pip_install.rs`, `install_console_script_in_tab_path` and `install_wheel_script_in_tab_path`: now snapshot successful direct execution and the generated shell wrapper, retain explicit-Python execution checks, and assert that spaces and excessive path length cannot mask the tab condition. Their Linux gates and the module's `test-python`/`test-pypi` gates remain intact.

- `crates/uv-install-wheel/src/wheel.rs`, `test_shebang`: checks direct shebangs, space-containing paths, relocation, Windows, quotes, and long paths; no tab case.
- `crates/uv/tests/pip_install/pip_install.rs`, `install_wheel_script_with_shell_launcher_preserves_encoding_cookie`: creates `venv with spaces`, installs a wheel data script with CRLF and a Latin-1 declaration, directly executes it, and snapshots successful `café` output. This tests an already-selected shell wrapper and a wheel data script, not a generated console entrypoint under a short tab-only path.
- In the same integration module, `install_ascii_wheel_script_in_non_ascii_environment` executes an ASCII-declared wheel data script in `café env` and snapshots `ok`; the embedded space selects the wrapper.
- In the same integration module, `install_relocatable` installs a console entrypoint, verifies `Hello world!` before and after moving the environment, and checks a Unix symlink. It explicitly enables relocation and does not test the missing tab trigger.

During the original reproduction, these repository tests were inspected rather than executed, and the three targeted environments and direct-interpreter control used the installed uv. The subsequent fix validation uses the checkout's debug build, as detailed below.

## Related

- astral-sh/uv-dev#3064 — Parent regression pull request. Adds the console-entrypoint and wheel-data-script tab-path tests; both originally asserted `NotFound` and now assert successful execution.
- astral-sh/uv#2076 — Support paths with spaces in launchers (closed; issue). Historical counterpart: POSIX console launchers failed because the kernel split interpreter paths containing spaces. It requested the shell-wrapper fallback, implemented by astral-sh/uv#2097. Its space-path case was fixed; the newly reported tab condition remains outside that fix.
- astral-sh/uv#2097 — Wrap unsafe script shebangs in `/bin/sh` (merged; pull request). Merged on 2024-02-29 and closed astral-sh/uv#2076. The diff introduced the same fallback predicate still present today: shebang length over 127 bytes or a literal space. Tabs were never included, establishing an omitted case rather than a regression of tab support.
- astral-sh/uv#22323 — `uv run --with` packages invisible to project entrypoints when the venv path forces the /bin/sh shebang wrapper (open; issue). Adjacent issue involving the same generated wrapper. Its report and follow-up describe valid wrappers for long or space-containing paths being skipped when copying entrypoints into an ephemeral environment. Here, a short tab-containing path does not receive a wrapper at all and the launcher cannot start; these require different fixes.


## Supporting evidence

- `crates/uv-install-wheel/src/wheel.rs:128`, `format_shebang`, computes the full shebang length as `2 + executable.len() + 1`. Before the fix, its fallback condition was `shebang_length > 127 || executable.contains(' ') || relocatable`. A tab alone did not select the wrapper, matching the original observed launcher. The fix changes the character check to `executable.contains([' ', '\t'])`.
- `crates/uv-install-wheel/src/wheel.rs:385` connects `format_shebang` to generated console entrypoints.
- The wrapper branch single-quotes the interpreter and calls `escape_posix_for_single_quotes` from `crates/uv-shell/src/shlex.rs:33`. The working space-plus-tab control confirms that a tab can be represented correctly in this wrapper. It does not establish behavior for every possible destination string.
- The historical diff of astral-sh/uv#2097 introduced the literal-space and length checks without a tab check. It merged on 2024-02-29 and was also identified as the fix for astral-sh/uv#1466.
- The reporter-linked pypa/virtualenv commit `2519941bed33ee0d3ac99277f67913e439e476a9`, dated 2026-10-17, adds tab-path launcher handling alongside interpreter-path quoting changes. A change in virtualenv does not establish identical security impact in uv.
- astral-sh/uv#22526, merged on 2026-10-14 and listed under uv 0.13.2 in `CHANGELOG.md`, fixes astral-sh/uv#21276 by preserving wheel data-script encoding declarations and protecting paths from source-code decoding. That fix concerns another script-generation path, not the tab trigger for generated console entrypoints.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separately searched tab-path launch failures and shell interpretation: bad interpreter, shebang, console_scripts, tab/venv, CVE-2026-113722, format_shebang, format_shell_shebang, whitespace, special characters, quoting, escaping, and command substitution, including comments and installer/environment labels. Removed version and platform restrictions. PR keyword searches returned no hits, so also screened the latest 1,000 PRs across states and directly inspected referenced historical fixes. Followed astral-sh/uv#1466 to astral-sh/uv#2097. Ruled out astral-sh/uv#2744 and astral-sh/uv#8035 (setuptools build-interpreter paths), astral-sh/uv#8947 (activation quoting), astral-sh/uv#9348 (upstream bundled scripts), astral-sh/uv#16209 (BusyBox realpath), and astral-sh/uv#21276 with astral-sh/uv#22526 (wheel data-script encoding). Also inspected the reporter-linked virtualenv commit; no existing tab-specific uv fix or duplicate was found.

The strongest excluded matches differ in the failure mechanism:

- astral-sh/uv#2744 and its duplicate astral-sh/uv#8035 describe setuptools editable scripts pointing to a temporary build interpreter. The discussion explicitly distinguishes those scripts from console entrypoints.
- astral-sh/uv#8947 concerns apostrophes in activation scripts, not console launcher generation.
- astral-sh/uv#9348 concerns shell wrappers shipped in python-build-standalone; maintainer comments direct the fix upstream.
- astral-sh/uv#16209 concerns BusyBox interpreting `realpath --` incorrectly in relocatable wrappers. Maintainer comments state that the launcher still functions despite the warning.
- astral-sh/uv#21276 and astral-sh/uv#22526 concern Python source encoding after insertion of a multi-line wrapper, not interpreter-path splitting by the kernel.

Searches covered both the literal tab/error report and the title's broader shell-syntax claim. Conceptual searches used whitespace, special characters, launcher quoting, escaping, and command substitution; searches using source identifiers and historical fixes were assessed separately. Searches were not restricted to Fedora, a package name, or uv 0.13.6. PR keyword searches returned no results even for known historical fixes, so the handoff also relies on direct linked-PR inspection and screening the latest 1,000 PRs across states; this is not a claim that every historical PR was enumerated.

## Fix

**Outcome: fixed in the checkout.** Only `crates/uv-install-wheel/src/wheel.rs` and `crates/uv/tests/pip_install/pip_install.rs` are modified.

### Implementation and scope

`format_shebang` now treats a literal tab like a space when selecting the POSIX `/bin/sh` launcher. The existing quoting implementation handles the tab-containing interpreter path. Length-based fallback, relocation, and non-POSIX behavior are unchanged. Both console-entrypoint generation and wheel data-script rewriting call this function, so the single predicate correction fixes both demonstrated manifestations.

The two regressions from astral-sh/uv-dev#3064 retain their local wheel fixtures, installation snapshots, full launcher snapshots, and explicit-Python execution controls. They now require successful direct execution instead of `NotFound`, and expect the quoted shell wrapper. Fixture assertions ensure that the interpreter path has no spaces and its direct shebang would be at most 127 bytes, isolating the tab condition. No new test or feature gate was needed.

Neighboring encoding-cookie, non-ASCII-path, source-decoding, relocation, and shebang-argument tests were inspected. The separate entrypoint producer/consumer in `crates/uv-project-commands/src/run.rs` and its `run_with_overlay_interpreter` integration test were also inspected. The latter covers installed entrypoints copied into an overlay, including relocatable launchers. Its absolute-shell-wrapper recognition limitation is the separate cause described by astral-sh/uv#22323; this change does not expand into that issue. No additional fixture or assertion contradicting this tab-selection fix was identified in that module.

### Validation

Validation used the checkout's unoptimized debug/test profile on Linux with CPython 3.12.16:

1. Before any edits, `cargo test -p uv --test pip_install in_tab_path -- --nocapture` passed both original regressions, which asserted `NotFound`.
2. After changing only the tests to require successful launch, the same command failed both tests with `Failed to spawn .../tab\tdir/venv/bin/tab-probe: No such file or directory (os error 2)`. Installation still succeeded. This established the expected failure before modifying production code.
3. After the predicate correction, the same command passed both updated regressions, including generated-file snapshots and direct and explicit-Python execution.
4. Six existing installer integration tests passed together: `install_crlf_wheel_script_preserves_encoding_cookie`, `install_wheel_script_with_shell_launcher_preserves_encoding_cookie`, `install_ascii_wheel_script_in_non_ascii_environment`, `install_wheel_script_does_not_execute_decoded_interpreter_path`, `install_relocatable`, and `strip_shebang_arguments`. Command: `cargo test -p uv --test pip_install -- pip_install::install_crlf_wheel_script_preserves_encoding_cookie pip_install::install_wheel_script_with_shell_launcher_preserves_encoding_cookie pip_install::install_ascii_wheel_script_in_non_ascii_environment pip_install::install_wheel_script_does_not_execute_decoded_interpreter_path pip_install::install_relocatable pip_install::strip_shebang_arguments --exact`.
5. `cargo test -p uv --test project run::run_with_overlay_interpreter -- --exact` passed the existing producer/consumer round trip.
6. `RUSTUP_HOME=/tmp/uv23057-rustup cargo fmt --all` and `git diff --check` succeeded. The supplied toolchain lacked rustfmt and was read-only, so Rust 1.99.0 with rustfmt was installed under a temporary directory for formatting.

No full test suite or release build was run. Fedora and other platforms were not tested; the Linux regressions remain platform-gated. No arbitrary shell-syntax execution or security impact is claimed. No commits, Git configuration changes, or GitHub changes were made.

Pull request: https://github.com/astral-sh/uv-dev/pull/3067
