# activate.bat sets wrong VIRTUAL_ENV when the path contains an apostrophe

Issue: astral-sh/uv#23149

Classification: bug

## Summary

The reporter uses uv 0.13.7 (316c827e0, November 3, 2026) and CPython 3.15.2 on Windows 11 x86_64. After creating `.venv` under `C:\Tools\it's a test`, running `.venv\Scripts\activate.bat` sets `VIRTUAL_ENV` to `C:\Tools\it'"'"'s a test\.venv`. The expected value is the actual environment directory, with the original apostrophe. The report says activation fails.

The malformed batch environment value was reproduced using the installed uv 0.13.6 on Linux with CPython 3.12.3, then executing its unmodified generated `activate.bat` under Wine 9.0. The batch variable contained the exact reported apostrophe expansion and failed to locate the environment's `pyvenv.cfg`. A control path containing spaces but no apostrophe worked. POSIX activation of the apostrophe-containing environment also worked. Native Windows 11, the Windows uv binary, uv 0.13.7, and Python 3.15.2 were not executed.

The pre-fix generated artifact and implementation agree: ordinary, non-relocatable batch activation received a path processed by the POSIX single-quote escaping helper. The focused fix below bypasses that helper for ordinary batch activation. Historical context links this shared fallback to astral-sh/uv#8984, which fixed the POSIX activation issue astral-sh/uv#8947. Closed, unmerged astral-sh/uv#9466 proposed separate batch handling but did not ship a fix.

## Classification

This is a Windows batch activation correctness bug: a valid apostrophe-containing environment path is rewritten to a different string. The batch template also uses `VIRTUAL_ENV` to construct `PATH`, so the incorrect value affects interpreter lookup. The reproduction directly checked the invalid variable and environment configuration lookup; it did not execute a Windows Python interpreter.

The report does not identify a last known-good release or claim a regression immediately after an upgrade. The existing historical investigation attributes the shared POSIX escaping fallback to astral-sh/uv#8984, released in uv 0.5.3. The current reproduction demonstrates the behavior in installed uv 0.13.6; it does not independently establish the first affected release. There is no evidence of a shipped batch-specific apostrophe fix that was later reverted. The unmerged double-quote proposal astral-sh/uv#9466 is not evidence of a released fix.

## Reproduction

**Outcome: reproducible**, with a platform/version qualification: the installed Linux uv generated the batch activator, and Wine executed it. This is observed generated-file and batch runtime behavior, rather than a conclusion based solely on source inspection. An exact native Windows 11 run remains unverified.

### Environment and isolation

- Host: Linux x86_64, Ubuntu 24.04 environment; kernel `7.0.0-1017-azure`.
- Installed executable on `PATH`: `/opt/hostedtoolcache/uv/0.13.6/x86_64/uv`, reporting `uv 0.13.6 (x86_64-unknown-linux-gnu)`.
- Interpreter: CPython 3.12.3, selected explicitly using the existing `python` executable. No Python downloads or package installation were needed for the environments.
- Batch runtime: Wine 9.0 (`9.0~repack-4build3`, Ubuntu amd64 packages), extracted under the temporary reproduction directory without a system installation. Its dependencies and Windows system32 files were supplied within that directory. PRoot 5.1.0 redirected Wine's `/run/user/1001` runtime files into the same temporary tree. `WINEPREFIX` and reproduction caches were isolated there. The generated activation files were not edited.
- Reproduction artifacts: `/tmp/uv-23149-3ns2732b`. Configuration discovery was disabled; uv used an explicit Python interpreter, `--no-project`, and `--offline`. The subprocess environments contained only the needed runtime settings, with no inherited authentication settings.

### Minimal environment creation

The essential command is `uv venv` in a directory containing an apostrophe. The isolated equivalent used for this run was:

```sh
repro_dir=$(mktemp -d /tmp/uv-23149-XXXXXX)
mkdir "$repro_dir/it's a test"
cd "$repro_dir/it's a test"
TMPDIR="$repro_dir" UV_CACHE_DIR="$repro_dir/cache" \
UV_PYTHON_INSTALL_DIR="$repro_dir/python" UV_PYTHON_DOWNLOADS=never UV_NO_CONFIG=1 \
uv venv --no-project --offline --python "$(command -v python)"
```

Creation exited successfully using CPython 3.12.3. Linux uv also generates a batch activator; it is under `.venv/bin/activate.bat`, rather than the Windows `.venv\Scripts\activate.bat`. The generated assignment was:

```bat
@for %%i in ("/tmp/uv-23149-3ns2732b/it'"'"'s a test/.venv") do @set "VIRTUAL_ENV=%%~fi"
```

The actual directory contains one ordinary apostrophe. The generated file contains the POSIX sequence `'"'"'` in its place.

### Batch execution and observed result

Wine maps the Linux filesystem through `Z:`. The explicit batch probe called the unmodified generated activator and printed the variable, then checked the environment configuration file:

```bat
@echo off
set "PATH=C:\windows\system32;%PATH%"
call "Z:\tmp\uv-23149-3ns2732b\it's a test\.venv\bin\activate.bat"
echo EXPECTED=Z:\tmp\uv-23149-3ns2732b\it's a test\.venv
echo OBSERVED=%VIRTUAL_ENV%
if exist "%VIRTUAL_ENV%\pyvenv.cfg" (echo CONFIG_EXISTS=yes) else (echo CONFIG_EXISTS=no)
```

The isolated driver invoked Wine's `cmd.exe /d /c` on this probe. It is retained as `run-batch.py` and can be run with `python /tmp/uv-23149-3ns2732b/run-batch.py check-apostrophe.cmd`. Final output, with exit code 0, was:

```text
EXPECTED=Z:\tmp\uv-23149-3ns2732b\it's a test\.venv
OBSERVED=Z:\tmp\uv-23149-3ns2732b\it'"'"'s a test\.venv
CONFIG_EXISTS=no
```

A separate environment was created with the same installed uv command under `its a test`, retaining the spaces and removing the apostrophe. The corresponding explicit probe (`check-control.cmd`) produced:

```text
EXPECTED=Z:\tmp\uv-23149-3ns2732b\its a test\.venv
OBSERVED=Z:\tmp\uv-23149-3ns2732b\its a test\.venv
CONFIG_EXISTS=yes
```

The control establishes that the Wine path mapping and configuration-file check work for the same generated batch template. Separately, sourcing `.venv/bin/activate` in Bash for the apostrophe-containing environment succeeded: both `os.environ["VIRTUAL_ENV"]` and `sys.prefix` were `/tmp/uv-23149-3ns2732b/it's a test/.venv`.

### Existing test coverage and limits

Relevant integration tests and their actual setup/assertions were read in `crates/uv/tests/python/venv.rs`:

- `create_venv_apostrophe` creates an environment named `Testing's`, checks the POSIX activation instruction, sources `bin/activate` in a shell, and asserts that Python's `sys.prefix` matches the environment directory. It has `#[cfg(target_os = "linux")]`; it does not execute `activate.bat` or check cmd's `VIRTUAL_ENV`. Parent regression pull request astral-sh/uv-dev#3117 extended it with a snapshot of the generated batch assignment; the fix updates that snapshot to require a literal apostrophe. The module is gated on `test-python` in `crates/uv/tests/python/main.rs`.
- `verify_pyvenv_cfg_relocatable` (approximately lines 1836–1912) creates an environment with `--relocatable` and checks the generated batch line using `%~dp0..`. It does not use an apostrophe-containing path or execute batch activation, and it exercises a different template-dispatch branch from this report.
- `create_venv_powershell_unc` uses Windows PowerShell and a UNC path containing a space, then checks `VIRTUAL_ENV` and `uv python find`. It does not cover batch apostrophe handling.

The original reproduction investigation found no matching ordinary batch apostrophe activation regression test under `crates/uv/tests/` or `crates/uv-client/tests/it/` and ran no Rust tests or builds. The parent regression and focused debug-profile fix validation are documented below. No last known-good version comparison was performed because the report does not supply one. Native Windows confirmation should execute the generated batch file and check both the exact `VIRTUAL_ENV` and the selected Python executable; the present reproduction establishes the malformed variable and invalid environment path under Wine.

## Implementation evidence

- `crates/uv-virtualenv/src/virtualenv.rs`, `create`: before the fix, only relocatable `activate.bat` had a dedicated path branch, and ordinary batch activation reached `escape_posix_for_single_quotes(location_string)`. The installed executable and pre-fix debug build independently generated this transformation. The fixed dispatch borrows the unmodified location string for ordinary batch activation; the template substitution and relocatable branch are unchanged.
- `crates/uv-shell/src/shlex.rs:22–38`: the POSIX helper replaces an apostrophe with `'"'"'`, matching the observed generated file and runtime variable.
- `crates/uv-virtualenv/src/activator/activate.bat:29`: the batch `for` statement assigns the substituted path to `VIRTUAL_ENV`. Line 66 uses `%VIRTUAL_ENV%` to prepend the environment's script directory to `PATH`.
- `changelogs/0.5.x.md:248–310`: uv 0.5.3 includes the apostrophe-name change from astral-sh/uv#8984. This historical release entry does not establish native execution results for the present report.

## Related

The historical related-item states and GitHub search findings below are retained from the existing issue-context investigation. The parent regression pull request is identified by the supplied local metadata. No additional GitHub requests were made for the fix.

- astral-sh/uv-dev#3117 (parent regression pull request): adds a batch-assignment snapshot to `create_venv_apostrophe`. Its original snapshot records the malformed path and passes before the production fix. The fix retains this test and changes its expectation to the actual directory path.
- astral-sh/uv#8947 (closed; issue): historical apostrophe-path activation failure in uv 0.4.25 on Fedora, using `source .venv/bin/activate`. Maintainer discussion concerns POSIX quoting, and astral-sh/uv#8984 closed it. The present report concerns cmd.exe receiving POSIX escape syntax.
- astral-sh/uv#8984 (merged; pull request): Allow apostrophe in venv name. Merged November 15, 2024 and released in uv 0.5.3. The prior investigation found that its diff applies `escape_posix_for_single_quotes` to the default activation-template branch, including ordinary batch activation, and adds the Linux-only apostrophe test. That shared fallback was present in the pre-fix checkout; the fix excludes ordinary batch activation from it.
- astral-sh/uv#9466 (closed, unmerged; pull request): Escape path in batch script. Proposed a dedicated ordinary batch path branch bypassing POSIX escaping, but focused on double quotes during discussion of astral-sh/uv#9424. It was closed after the author noted that Windows filenames cannot contain double quotes. This was an abandoned proposal, not a shipped apostrophe fix or an open tracker for this failure.
- astral-sh/uv#11828 and merged astral-sh/uv#11831 concern non-ASCII paths decoded using the wrong Windows code page. The template already includes that code-page change. The ASCII apostrophe expansion reproduced here is a different failure.
- astral-sh/uv#1763 concerns extended-length Windows path prefixes in `PATH`.
- astral-sh/uv#23060 concerns fish executing commands from unescaped prompt/path input; the shell and observed failure differ.
- astral-sh/uv#15294 concerns broader activation-test coverage.

## Prior related-item search scope

The earlier investigation searched astral-sh/uv open and closed issues and open, closed, and merged pull requests for `activate.bat`, apostrophes, `VIRTUAL_ENV`, single quotes, escaping, batch activation, special characters, and `area:environments`. It inspected issue bodies, maintainer comments, timelines, reviews, diffs, and the uv 0.5.3 changelog. Historical pull requests were recovered through issue references after PR searches returned no hits. It followed astral-sh/uv#8947 to astral-sh/uv#8984 and astral-sh/uv#9424 to astral-sh/uv#9466. No existing open tracker for this exact Windows apostrophe failure was found within that search; a final broader query hit the GitHub API rate limit.

## Fix

**Outcome: fixed.** Ordinary `activate.bat` generation now uses the literal environment path inside the batch template's double quotes. The production change adds the `(false, "activate.bat")` dispatch branch in `crates/uv-virtualenv/src/virtualenv.rs`; it does not alter POSIX escaping, the batch template, or relocatable activation.

The parent regression in `crates/uv/tests/python/venv.rs`, `create_venv_apostrophe`, now snapshots an ordinary apostrophe in the batch assignment. Its original command-output snapshot and POSIX activation/Python `sys.prefix` round trip remain intact. The existing Linux gate and enclosing `test-python` feature gate remain appropriate.

The unchanged parent regression passed against the original implementation. Changing only its expectation to the desired literal path then failed exactly on `Testing's` versus `Testing'"'"'s` in the generated batch assignment. After adding the production branch, the updated regression passed.

The neighboring dispatch branches and activator consumers were reviewed. Relocatable batch activation uses `%~dp0..` and already has a correct assertion in `verify_pyvenv_cfg_relocatable`; that test passes without changes. PowerShell, xonsh, and `activate_this.py` determine the environment directory from the script location instead of consuming this interpolated path. The remaining batch templates do not interpolate `VIRTUAL_ENV_DIR`. The POSIX-shell templates retain their existing quoting. The relevant integration tests in the Python and pip-install modules exposed no additional assertion encoding this same ordinary-batch escaping error. No additional distinct failure justified a new test or broader production changes.

Successful focused validation:

- `INSTA_UPDATE=no cargo test -p uv --test python venv::create_venv_apostrophe -- --exact` — passes in the debug test profile after demonstrating the expected pre-fix failure. It covers batch generation and a real POSIX activation/Python round trip with CPython 3.12.16.
- `INSTA_UPDATE=no cargo test -p uv --test python venv::verify_pyvenv_cfg_relocatable -- --exact` — passes, including the existing batch `%~dp0..` assertion.
- Debug uv 0.13.7 generated an environment under `$TMPDIR/it's a test/.venv` with explicit CPython 3.12.16. Wine 9.0 executed the unmodified generated batch activator. Before the fix, `VIRTUAL_ENV` contained the unwanted POSIX apostrophe expansion and `CONFIG_EXISTS=no`. After regenerating with the fixed debug binary, expected and observed values both equal `Z:\home\runner\work\_temp\fix-bug\it's a test\.venv`, and `CONFIG_EXISTS=yes`. The explicit probe and its output are retained in `$TMPDIR/check-apostrophe.cmd` and `$TMPDIR/check-apostrophe.cmd.output.txt`, with the isolated runtime driver in `$TMPDIR/run-batch.py`.
- `cargo xwin clippy -p uv-virtualenv --all-targets --target x86_64-pc-windows-msvc --target-dir "$TMPDIR/windows-target" --locked -- -D warnings` — passes in the debug dev profile for the affected production crate. The check used a temporary Rust 1.99.0 toolchain, LLVM 18, and an isolated Windows SDK/cache.
- `cargo fmt --all` and `cargo fmt --all -- --check` — pass using Rust 1.99.0's rustfmt from a temporary toolchain installation.
- `git diff --check` — passes.

The batch runtime check uses Wine on Linux, not native Windows 11 or a Windows Python executable. It verifies the corrected variable and downstream configuration-file lookup. Native Windows Python executable selection remains unverified.

Checkout changes are limited to the affected production file and the original parent regression-test file. No manifests, lockfiles, automation files, Git configuration, or other tracked files were changed. No commits, pushes, or GitHub changes were made.

Pull request: https://github.com/astral-sh/uv-dev/pull/3118
