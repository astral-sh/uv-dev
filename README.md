# `uv pip compile -o /dev/null` replaces the /dev/null device with a regular file (atomic write over non-regular files)

Issue: astral-sh/uv#22487

Classification: bug

## Summary

The reporter describes `uv pip compile requirements.in -o /dev/null` replacing the null character device with a regular requirements file when run as root in a Docker container whose `/dev` is writable. The supplied reproduction uses a scratch character device (major/minor 1,3) and reports that it becomes a regular file. Reported environment: uv 0.13.0 installed through pip, Debian 13 x86_64, Linux 6.1.0-53-amd64, and Python 3.12.15.

The reported consequences are stale data when reading `/dev/null` and failed new tmux servers with `server exited unexpectedly`. These downstream effects are reported evidence, not independently reproduced here. Expected behavior is to write to a supported special destination or reject it without replacing it.

Source confirms an unguarded atomic replacement path shared by compile and export. The closest history is astral-sh/uv#6485 and astral-sh/uv#6487, with the mechanism introduced by astral-sh/uv#6274 and shared with export by astral-sh/uv#7109.

## Draft response

Your report matches a bug in the current output writer: it follows symlinks, then uses a temporary file and rename without checking whether the destination is a regular file. The fix in astral-sh/uv#6487 only covered symlinks. The next step is to handle or reject non-regular output destinations without replacing them, with regression coverage for pip compile and the shared uv export writer. To discard output in the meantime, omit `-o` and use `uv pip compile requirements.in > /dev/null`; this leaves warnings on stderr and avoids the atomic output-file path.

## Classification

The checked-out source confirms that OutputWriter::commit calls write_atomic without checking the destination's file type; on Unix the helper renames a temporary regular file over the destination. Where permissions and filesystem semantics permit, this can replace a device instead of writing to it. The historical symlink fix remains present and never covered device nodes, so this is neither a duplicate of that resolved issue nor an established recurrence of a previously fixed device bug.

The destructive filesystem behavior establishes a correctness bug without requiring a runtime reproduction. No existing issue or PR found provides a canonical discussion for device-node replacement. The report is not a request merely to expand the supported output formats.

## Related

- astral-sh/uv#6485 (issue, closed) — Respect output-file symlinks. The same command replaced an output symlink with a regular file after uv 0.3.1 introduced atomic writes; a maintainer confirmed this was unintended. Its fix covers symlinks, while this report concerns device nodes and other non-regular destinations.
- astral-sh/uv#6487 (pull request, merged) — Avoid overwriting symlinks in `pip compile` output. Fixed astral-sh/uv#6485 in uv 0.3.2 by resolving the output symlink before calling write_atomic. The inspected patch adds no destination file-type check, and that symlink handling remains present, so it does not establish a prior fix for device replacement.
- astral-sh/uv#6274 (pull request, merged) — Use atomic write for `pip compile` output. Introduced the buffered atomic output path in uv 0.3.1 to fix concurrent readers observing partially written requirements in astral-sh/uv#6239. This directly explains the current mechanism and why regular-file output should retain atomic writes; it did not address special files.
- astral-sh/uv#7109 (pull request, merged) — Add `--output-file` to `uv export`. Moved OutputWriter into shared command code and reused it for export. Current compile and export commands still share this unchecked atomic-write path, making export relevant to the requested audit; this PR added output-file support rather than fixing device handling.

## Supporting evidence

Source inspected at checkout commit `5cc7f15c359b83eed2c68c5578c1000bfd27e0da`:

- `crates/uv-pip-commands/src/compile.rs:619` constructs `OutputWriter` with the supplied output path, and line 807 commits it.
- `crates/uv-command-support/src/lib.rs:123` implements `OutputWriter::commit`. It creates the parent directory, tries `read_link`, strips terminal formatting, and invokes `uv_fs::write_atomic` at line 134. It does not inspect the destination's file type.
- `crates/uv-fs/src/lib.rs:487` creates a temporary file in the destination's parent, writes the content, and invokes `persist_with_retry`. Its non-Windows branch at line 724 calls `fs_err::rename(from, to)`. This confirms replacement rather than opening the device for output, subject to directory permissions and filesystem restrictions.
- `crates/uv-project-commands/src/export.rs:566` constructs the same writer. Export commits it at lines 446 and 493, establishing a second user-supplied-output call path to review.
- `crates/uv/tests/pip_compile/pip_compile.rs:15486` contains an existing Unix symlink-preservation test. It checks an output link targeting a regular file; it does not exercise a character device. Targeted inspection of this test module found no device/FIFO output-preservation test.
- `changelogs/0.3.x.md:174` records astral-sh/uv#6274 in uv 0.3.1; line 210 records astral-sh/uv#6487 in uv 0.3.2. The former PR replaces direct output-file writes with buffered atomic writes to resolve astral-sh/uv#6239; the latter only adds `read_link` handling. Both predate the reported uv 0.13.0.

## Scope and limitations

The source confirms the missing guard and replacement mechanism. No reproduction, build, or test was run, and no device was modified. Actual handling of `/dev/stdout`, FIFOs, sockets, and Windows `NUL` was not verified. These are additional destination classes requested for review, not independently established command reproductions. In particular, compile also reads an existing output path for resolution preferences at `crates/uv-pip-commands/src/compile.rs:470`, so supporting pipes requires considering the read path as well as the final write.

The issue timeline links omnigent-ai/omnigent#9743, which reports the same incident and requests better downstream tmux diagnostics. It points back to astral-sh/uv#22487 for the uv defect and is not a replacement for this discussion.

## Suggested next steps

- Define handling for non-regular output destinations before committing output: either write through supported destinations or return a clear error without replacing them. Keep atomic writes for regular files, which protect concurrent readers.
- Add focused regression coverage using a scratch destination and review both compile and export. Check existing tests and platform/feature gates before adding coverage.
- Audit other user-controlled paths reaching the atomic-write helpers; do not assume every helper caller has the same externally reachable trigger.
- For the reporter's resolution-only use case, omit `-o` and redirect stdout through the shell: `uv pip compile requirements.in > /dev/null`. This bypasses the output-file rename path and retains stderr diagnostics.

## Search scope

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered /dev/null, /dev/stdout, NUL, FIFO, write_atomic, OutputWriter, and the tmux error. Conceptual searches covered non-regular/special files, character devices, named pipes, mkfifo, output replacement, symlinks, and export. Historical searches covered atomic writes and closed/merged compile-output fixes; inspected candidate comments, timelines, PR patches, and release notes. Ruled out astral-sh/uv#3701 (stdout suppression), astral-sh/uv#4199 (configuration lookup under /dev/null), and astral-sh/uv#1382 (disabling cache). The downstream cross-reference omnigent-ai/omnigent#9743 concerns terminal diagnostics for the reported incident. No existing discussion or fix covering device replacement was found.

Candidate bodies and comments were inspected for astral-sh/uv#6485, astral-sh/uv#3701, astral-sh/uv#7058, astral-sh/uv#6239, astral-sh/uv#4199, and astral-sh/uv#1382. Linked PRs astral-sh/uv#6487 and astral-sh/uv#7109 were followed through issue timelines; astral-sh/uv#6274 was located in release notes. All three PRs were inspected directly, including their patches and available discussion. PR searches returned no direct special-file fix.
