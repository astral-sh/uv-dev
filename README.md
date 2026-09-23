# Race condition in SIGPIPE causes network errors to kill uv run

Issue: astral-sh/uv#21935

Classification: bug

## Summary

On Linux amd64 with uv 0.12.8 and Python 3.13.5, the reporter intermittently sees `uv run --frozen -m modulename` exit with status 141 under high I/O load. Their proposed sequence is that a timed-out HTTP/2 repository connection causes a `writev` call to encounter `EPIPE`, uv receives SIGPIPE, uv forwards SIGPIPE to the newly spawned Python child before Python has installed its normal signal disposition, and the child terminates.

The checked-in Unix child runner confirms the central signal-forwarding behavior: it registers a Tokio SIGPIPE listener and unconditionally sends each received SIGPIPE to the child process. The merged diff for astral-sh/uv#13017 introduced both the registration and forwarding branch. The repository evidence does not by itself establish that an HTTP/2 timeout or `writev` is the source of the signal in this occurrence, so that part of the proposed mechanism still needs reproduction.

The reporter has now supplied a synthetic, network-independent integration-test reproduction. It redirects uv's verbose stderr to a closed Unix socket, then has the child send SIGWINCH to uv so uv attempts a debug write while the child is running. The resulting uv-owned SIGPIPE is forwarded to the child. The reporter states that the test passes after removing the `sigpipe_handle.recv` forwarding branch. This reproduction has not been run as part of this handoff update.

No existing issue or pull request tracks this exact failure. The closest history is astral-sh/uv#13017, which added broad signal forwarding to fix the opposite failure reported in astral-sh/uv#12830, where externally delivered signals terminated uv without reaching its child. astral-sh/uv#11886 is adjacent because it explains the observed status 141: uv currently converts a signal-terminated child into a normal-looking 128+signal exit code, but that issue does not cover why the child received SIGPIPE.

A repository member identified two possible implementation directions: stop forwarding SIGPIPE, or replace uv with the Python process via `exec` in cases that do not require uv to retain a child process. These are proposals rather than a settled maintainer decision.

## Reproduction

The proposed regression test is Unix-only and fits alongside the existing project-run integration tests:

1. Create a `UnixStream::pair`, close the peer, and use the remaining endpoint as stderr for a verbose uv command.
2. Run `uv -vv run --no-project -- sh -c 'trap - PIPE; kill -WINCH "$PPID"; sleep 1; echo survived'`.
3. The child restores the default SIGPIPE disposition and sends SIGWINCH to its uv parent.
4. Handling SIGWINCH produces a uv debug log directed at the closed socket, generating SIGPIPE in uv while the child is alive.
5. The current SIGPIPE branch forwards that signal to the child. The intended regression snapshot expects exit code 0 and `survived` on stdout.

The reporter says the proposed test fails with the current forwarding branch and passes when the `sigpipe_handle.recv` arm is removed. The test was supplied as a patch against `crates/uv/tests/project/run.rs`, uses the existing `uv_snapshot!` style, and is gated with `#[cfg(unix)]`. These results are user-reported and have not been independently executed here.

## Classification

This is a bug, not a duplicate. The current source unconditionally forwards SIGPIPE received by the uv parent to the spawned child. Although forwarding externally delivered signals is intentional, an incidental SIGPIPE arising from uv's own I/O is not a signal intended for the child; delivering it can terminate an otherwise healthy command. The original HTTP/2 timing has not been independently validated, but the new synthetic reproduction isolates the same source-confirmed correctness problem without depending on network behavior.

astral-sh/uv#12830 and astral-sh/uv#13017 are historical context rather than canonical trackers for this report. They address signals that should reach the child but did not. No open issue or pull request already tracks the newly reported failure mode, and no merged fix for this specific behavior was found.

## Maintainer direction

A repository member proposed that uv could stop forwarding SIGPIPE. The stated compatibility cost is that an explicitly targeted `kill -PIPE <uv PID>` would no longer reach the child. Ordinary shell pipelines such as `uv run ... | consumer` would still deliver SIGPIPE directly to the writing child when the reader closes the pipe. The same comment cautions that correlating an asynchronously received SIGPIPE with a particular `EPIPE` is not reliable, so selectively suppressing only signals attributed to uv's own failed write is not considered a practical approach.

The reporter confirms that removing the `sigpipe_handle.recv` select arm makes the proposed synthetic regression test pass. This demonstrates the effect of that change but does not resolve the compatibility decision about explicitly forwarding SIGPIPE.

The alternative proposal is to use `exec` for eligible Python invocations so uv is no longer a signal-forwarding parent. The member noted that uv must retain the forked-process model for cases such as `--isolated`; the scope and implementation cost of using `exec` elsewhere are still unknown. A separate cleanup-holder process was mentioned but considered difficult to implement correctly. No fix has been selected.

## Related

- astral-sh/uv#13017 — Merged pull request, “Forward additional signals to the child process in `uv run`.” Its diff added the SIGPIPE listener and unconditional child forwarding that match the source-backed portion of this report. It fixed the opposite problem of signals not reaching children.
- astral-sh/uv#12830 — Closed bug, “Unexpected behavior when handling signals.” This was closed by astral-sh/uv#13017 and establishes the reason broad forwarding was added. Its symptom was orphaned children after externally delivered signals terminated uv, not uv-generated SIGPIPE terminating a child.
- astral-sh/uv#11886 — Open bug, “Handling of signal exit from subprocess is incorrect.” It tracks uv mapping signal termination to 128+signal without the shell's signal diagnostic, which accounts for status 141 after a SIGPIPE death. It does not track unintended SIGPIPE forwarding.
- astral-sh/uv#3095 — Open enhancement, “Consider `execv` for `uv run` on Unix.” It is broader than this bug, but provides the existing design discussion for the member's proposal to avoid the parent/child signal proxy by using `exec` where cleanup and invocation mode permit it.

## Search evidence

Literal searches across open and closed issues and open, closed, and merged pull requests covered `SIGPIPE`, `EPIPE`, `writev`, “broken pipe,” “exit code 141,” “status 141,” “signal 13,” HTTP/2 timeouts, and network-triggered `uv run` termination. Conceptual searches covered signal forwarding and propagation, child processes, process groups, Unix/Tokio signal handling, and `uv run`. Fix-oriented searches inspected closed issues, merged pull requests, referenced discussions, and the history around astral-sh/uv#13017 and astral-sh/uv#12830.

astral-sh/uv#12244 was a plausible literal match because it discusses SIGPIPE and broken pipes, but it concerns shell-completion output writing to a closed stdout pipe and uv panicking; it does not involve network I/O or forwarding a signal to an `uv run` child. astral-sh/uv#3095 is not a tracker for this failure, but the new maintainer proposal makes its broader Unix `exec` design discussion relevant implementation background.

## Supporting source evidence

In `crates/uv/src/child.rs`, `run_to_completion` installs `handle_signal(SignalKind::pipe())`. Its select loop logs `Received SIGPIPE, forwarding to child` and invokes `signal::kill(child_pid, signal::Signal::SIGPIPE)` without checking whether the signal was externally addressed to uv or arose from uv's own I/O. When the child then terminates from signal 13, the same function maps that disposition to 128+13, yielding exit status 141. The astral-sh/uv#13017 diff shows that both SIGPIPE handling branches were introduced together.

The proposed integration test provides a deterministic trigger for that source path: SIGWINCH prompts uv to log to a deliberately closed stderr socket, causing SIGPIPE in uv while a child with the default SIGPIPE disposition is running. This removes the network stack and HTTP/2 timing from the minimal reproduction, while preserving the reported failure mechanism.
