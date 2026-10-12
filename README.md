# `uv pip compile -o /dev/null` replaces the /dev/null device with a regular file (atomic write over non-regular files)

Issue: astral-sh/uv#22487

Classification: bug

## Summary

The reporter describes `uv pip compile requirements.in -o /dev/null` replacing the null character device with a regular requirements file when run as root in a Docker container whose `/dev` is writable. The supplied reproduction uses a scratch character device (major/minor 1,3) and reports that it becomes a regular file. Reported environment: uv 0.13.0 installed through pip, Debian 13 x86_64, Linux 6.1.0-53-amd64, and Python 3.12.15.

The reported consequences are stale data when reading `/dev/null` and failed new tmux servers with `server exited unexpectedly`. These downstream effects are reported evidence, not independently reproduced here. Expected behavior is to write to a supported special destination or reject it without replacing it.

Runtime reproduction with the installed uv 0.12.13 confirms replacement of a non-regular output destination: a disposable FIFO became a regular requirements file, and a syscall trace recorded a successful rename over it. The exact character-device case was not tested because this environment denies scratch character-device creation. Debug builds of the uv 0.13.0 checkout also reproduced FIFO replacement in both parent regression tests. The fix rejects special output destinations before atomic replacement, including FIFO targets reached through symlinks. The closest history is astral-sh/uv#6485 and astral-sh/uv#6487, with atomic output introduced by astral-sh/uv#6274 and shared with export by astral-sh/uv#7109.

## Classification

Bug; reproduction outcome: **reproducible**, specifically the reported atomic replacement of a non-regular output file. A targeted runtime test replaced a FIFO with a regular file and returned success. The observed rename agrees with the atomic-write path in the source before the fix. This confirms a filesystem correctness issue; it does not independently establish the reported character-device or tmux consequences.

The historical symlink fix remains present and does not cover device nodes or FIFOs, so this is neither a duplicate of that resolved issue nor an established recurrence of a previously fixed device bug. No existing issue or PR found provides a canonical discussion for device-node replacement.

## Reproduction

### Environment and outcome

- Outcome: **reproducible** for replacement of a non-regular output destination, using a disposable FIFO explicitly within the report's scope.
- Installed executable: `/opt/hostedtoolcache/uv/0.12.13/x86_64/uv`, reporting `uv 0.12.13 (x86_64-unknown-linux-gnu)`.
- Python: `/usr/bin/python3`, version 3.12.3.
- Platform: Ubuntu 24.04.5 LTS, Linux 6.17.0-1022-azure, x86_64; normal user UID 1001.
- Fixture: `req.in` containing exactly `six==1.16.0` followed by a newline, with no project configuration. Resolution used public PyPI, disabled configuration discovery, and disabled Python downloads.
- Files, cache, configuration directories, and trace artifacts were isolated under `/tmp/uv-22487-dti61rgd`. The installed-binary reproduction did not modify checkout files or existing user state.

The reporter used uv 0.13.0, Python 3.12.15, and Debian 13 as root in Docker. The installed-binary reproduction did not use those exact versions or a character-device destination. The fix validation additionally exercised a debug build of the uv 0.13.0 checkout. Creating a disposable character device with major/minor 1,3 failed with `Operation not permitted`, including in a private user/mount namespace; privilege escalation was also unavailable. The real `/dev/null` was never used as a compile output destination.

### Minimal commands

The successful test is equivalent to the following shell sequence, with all paths in a new temporary directory:

```sh
scratch=$(mktemp -d /tmp/uv-22487-XXXXXX)
cd "$scratch"
mkdir tmp
export TMPDIR="$scratch/tmp"
export UV_CACHE_DIR="$scratch/cache"
export UV_PYTHON_INSTALL_DIR="$scratch/python"
export UV_PYTHON_DOWNLOADS=never
export UV_PYTHON=/usr/bin/python3
export UV_NO_CONFIG=1
printf 'six==1.16.0\n' > req.in
mkfifo -m 600 output.fifo
stat -c '%F %s' output.fifo
( : > output.fifo ) &
uv pip compile req.in --no-deps -o output.fifo
wait
stat -c '%F %s' output.fifo
cat output.fifo
```

The actual invocation used a clean subprocess environment with credential and XDG directories redirected into the scratch directory, a 45-second timeout, and a background Python thread that opened and closed the FIFO's writing end. This thread is equivalent to `( : > output.fifo ) &`: compile reads an existing output file for resolution preferences, so the writer supplies immediate EOF, as a null device would. No `--upgrade` or other option changing that read path was used.

Observed results:

```text
Before: FIFO, mode prw-------, size 0
Command: uv pip compile req.in --no-deps -o output.fifo
Exit: 0
Stderr: Resolved 1 package in 93ms
After: regular file, mode -rw-r--r--, size 147
```

The resulting regular file and stdout both contained:

```text
# This file was autogenerated by uv via the following command:
#    uv pip compile req.in --no-deps -o output.fifo
six==1.16.0
    # via -r req.in
```

Tracing only rename syscalls with `strace -f -e trace=rename,renameat,renameat2` recorded:

```text
rename("/tmp/uv-22487-dti61rgd/.tmpH3CLji", "output.fifo") = 0
```

This directly confirms that uv replaced the FIFO by renaming a temporary file over it. Ordinary user permissions were sufficient because the containing scratch directory was writable.

### Workaround check

A separate disposable FIFO with a reader attached was used for `uv pip compile req.in --no-deps > redirect.fifo`. The command exited 0, the reader received the compiled requirements, and `redirect.fifo` remained a FIFO. Omitting `-o` and redirecting stdout therefore avoids the replacement path. For discarding output, `uv pip compile requirements.in > /dev/null` uses that approach and leaves diagnostics on stderr; this exact real-device command was not run.

### Existing test coverage

Read `crates/uv/tests/pip_compile/pip_compile.rs::symlink` (starting at line 15486). It creates a regular `requirements.txt`, creates a symlink pointing to it, compiles `anyio` through `--output-file requirements-symlink.txt`, snapshots successful output, and asserts that the symlink still targets the original path. It is gated with `#[cfg(not(windows))]`. It covers preservation of a symlink targeting a regular file, not a character device or FIFO.

`crates/uv/tests/pip_compile/pip_compile.rs::compile_invalid_output_file` checks rejection of `-o pyproject.toml` as an unsupported output format; it does not check filesystem object types. The parent regression pull request subsequently added `pip_compile::output_fifo` and `export::requirements_txt_output_symlink_to_fifo`. Both initially asserted successful replacement of the FIFO with a regular file. Both now snapshot a rejection error and assert FIFO preservation; the export test also verifies the symlink target. Compile retains its Unix gate, and export retains its Unix and `test-universal` gates, in addition to their modules’ Python and PyPI gates. Focused test results are listed under Fix.

## Related

- astral-sh/uv-dev#2596 (pull request, open) — Parent regression tests for compile output to a FIFO and export output through a symlink to a FIFO. Both tests are retained and updated to assert rejection without replacement.

- astral-sh/uv#6485 (issue, closed) — Respect output-file symlinks. The same command replaced an output symlink with a regular file after uv 0.3.1 introduced atomic writes; a maintainer confirmed this was unintended. Its fix covers symlinks, while this report concerns device nodes and other non-regular destinations.
- astral-sh/uv#6487 (pull request, merged) — Avoid overwriting symlinks in `pip compile` output. Fixed astral-sh/uv#6485 in uv 0.3.2 by resolving the output symlink before calling write_atomic. The inspected patch adds no destination file-type check, and that symlink handling remains present, so it does not establish a prior fix for device replacement.
- astral-sh/uv#6274 (pull request, merged) — Use atomic write for `pip compile` output. Introduced the buffered atomic output path in uv 0.3.1 to fix concurrent readers observing partially written requirements in astral-sh/uv#6239. This directly explains the current mechanism and why regular-file output should retain atomic writes; it did not address special files.
- astral-sh/uv#7109 (pull request, merged) — Add `--output-file` to `uv export`. Moved OutputWriter into shared command code and reused it for export. Compile and export share this output writer, which is guarded by this fix; this PR added output-file support rather than fixing device handling.

## Supporting evidence

Implementation before the fix, inspected at checkout commit `5cc7f15c359b83eed2c68c5578c1000bfd27e0da`:

- `crates/uv-pip-commands/src/compile.rs:619` constructs `OutputWriter` with the supplied output path, and line 807 commits it.
- `crates/uv-command-support/src/lib.rs:123` implements `OutputWriter::commit`. It creates the parent directory, tries `read_link`, strips terminal formatting, and invokes `uv_fs::write_atomic` at line 134. It does not inspect the destination's file type.
- `crates/uv-fs/src/lib.rs:487` creates a temporary file in the destination's parent, writes the content, and invokes `persist_with_retry`. Its non-Windows branch at line 724 calls `fs_err::rename(from, to)`. This confirms replacement rather than opening the device for output, subject to directory permissions and filesystem restrictions.
- `crates/uv-project-commands/src/export.rs:566` constructs the same writer. Export commits it at lines 446 and 493, establishing a second user-supplied-output call path to review.
- `crates/uv/tests/pip_compile/pip_compile.rs:15486` contains an existing Unix symlink-preservation test. It checks an output link targeting a regular file; it does not exercise a character device. Targeted inspection of this test module found no device/FIFO output-preservation test.
- `changelogs/0.3.x.md:174` records astral-sh/uv#6274 in uv 0.3.1; line 210 records astral-sh/uv#6487 in uv 0.3.2. The former PR replaces direct output-file writes with buffered atomic writes to resolve astral-sh/uv#6239; the latter only adds `read_link` handling. Both predate the reported uv 0.13.0.

## Fix

Outcome: **fixed**. The checkout contains a focused production change in `crates/uv-command-support/src/lib.rs` and updates to both parent regression files, `crates/uv/tests/pip_compile/pip_compile.rs` and `crates/uv/tests/project/export.rs`.

`OutputWriter::commit` resolves the output symlink as before, then checks destination metadata before calling `write_atomic`. Existing special files (neither regular files nor directories) return an `InvalidInput` error identifying the path: `Cannot overwrite non-regular file ...`. This implements the report's accepted alternative of refusing a special destination. Regular and missing output files continue through atomic replacement; directories continue through the existing error path. The atomic helper itself and unrelated callers are unchanged.

The confirmed cause was applying a temporary-file rename to an existing special destination. The initial syscall trace demonstrated this, and both parent tests passed before modification while explicitly asserting FIFO replacement. After changing their expectations to rejection and FIFO preservation, both failed against the unchanged production code: the command still exited 0 instead of 2 and emitted no rejection error. With the guard, both pass. The export regression retains its existing symlink setup and checks that both the link and FIFO target survive. The invalid regular-file content snapshots were replaced by filesystem-type assertions because rejected FIFOs must not become readable requirements files.

The neighboring regular-file output and symlink tests, compile's existing-output preference reader, and export's writer-only path were inspected. The parent tests already cover the directly demonstrated cases; no additional test scenarios or command variants were added. Compile still reads existing output for preferences, so its FIFO fixture supplies EOF before reaching the guarded write. This change does not add general pipe streaming support.

Successful focused validation, using debug/test profiles:

- `cargo test -p uv --test pip_compile pip_compile::output_fifo -- --exact`: 1 passed.
- `cargo test -p uv --test project export::requirements_txt_output_ -- --nocapture`: 2 passed, covering FIFO-target rejection and ordinary requirements-file output.
- `cargo test -p uv --test pip_compile -- pip_compile::empty_output pip_compile::symlink pip_compile::upgrade_none --exact`: 3 passed, covering regular output, regular-file symlinks, and retained resolution preferences.
- A temporary-directory round trip using `target/debug/uv`: compiled `six==1.16.0` to `requirements.txt`, changed the input to `six>=1.16.0`, recompiled to the same output and retained 1.16.0, created a disposable virtual environment, then ran `pip sync` on that output. Importing six confirmed version 1.16.0. Artifacts: `/tmp/uv-22487-fixed-7k1juj5x`.
- `RUSTUP_TOOLCHAIN=stable cargo fmt --all` and its `-- --check` verification passed. The pinned 1.99.0 toolchain lacks rustfmt and its installation directory is read-only, so the installed stable formatter was used.
- `git diff --check` passed. Only the three allowed production/integration-test files changed in the checkout.

## Scope and limitations

FIFO behavior is verified on the installed uv 0.12.13 and on a debug build of the uv 0.13.0 checkout. The fixed debug build rejects both a direct FIFO output and the existing export fixture's symlink to a FIFO. Character devices, `/dev/stdout`, sockets, Windows `NUL`, and downstream tmux/glibc effects were not exercised. The real `/dev/null` was never used as a compile output destination, and no device was modified.

Compile reads an existing output path for resolution preferences before committing output; `crates/uv-resolve-operations/src/locked_requirements.rs` skips that read when upgrading all packages. A FIFO without a writer can therefore still block during that existing read. The fix prevents replacement once output reaches the writer; it does not redesign pipe reads, add direct writes to devices, or audit unrelated atomic-write callers. Concurrent replacement of the destination between metadata inspection and rename is also outside this focused change.

The issue timeline links omnigent-ai/omnigent#9743, which reports the same incident and requests better downstream tmux diagnostics. It points back to astral-sh/uv#22487 for the uv defect and is not a replacement for this discussion.

For resolution-only use, omit `-o` and redirect stdout through the shell: `uv pip compile requirements.in > /dev/null`. This bypasses the output-file rename path and retains stderr diagnostics.

## Search scope

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered /dev/null, /dev/stdout, NUL, FIFO, write_atomic, OutputWriter, and the tmux error. Conceptual searches covered non-regular/special files, character devices, named pipes, mkfifo, output replacement, symlinks, and export. Historical searches covered atomic writes and closed/merged compile-output fixes; inspected candidate comments, timelines, PR patches, and release notes. Ruled out astral-sh/uv#3701 (stdout suppression), astral-sh/uv#4199 (configuration lookup under /dev/null), and astral-sh/uv#1382 (disabling cache). The downstream cross-reference omnigent-ai/omnigent#9743 concerns terminal diagnostics for the reported incident. No existing discussion or fix covering device replacement was found.

Candidate bodies and comments were inspected for astral-sh/uv#6485, astral-sh/uv#3701, astral-sh/uv#7058, astral-sh/uv#6239, astral-sh/uv#4199, and astral-sh/uv#1382. Linked PRs astral-sh/uv#6487 and astral-sh/uv#7109 were followed through issue timelines; astral-sh/uv#6274 was located in release notes. All three PRs were inspected directly, including their patches and available discussion. PR searches returned no direct special-file fix.

Pull request: https://github.com/astral-sh/uv-dev/pull/2597
