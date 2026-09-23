# Race condition in SIGPIPE causes network errors to kill uv run

Issue: astral-sh/uv#21935

Classification: bug

## Summary

On Linux amd64 with uv 0.12.8 and Python 3.13.5, the reporter intermittently sees `uv run --frozen -m modulename` exit with status 141 under high I/O load. Their proposed sequence is that a timed-out HTTP/2 repository connection causes a `writev` call to encounter `EPIPE`, uv receives SIGPIPE, uv forwards SIGPIPE to the newly spawned Python child before Python has installed its normal signal disposition, and the child terminates.

The checked-in Unix child runner confirms the central signal-forwarding behavior: it registers a Tokio SIGPIPE listener and unconditionally sends each received SIGPIPE to the child process. The merged diff for astral-sh/uv#13017 introduced both the registration and forwarding branch. The repository evidence does not by itself establish that an HTTP/2 timeout or `writev` is the source of the signal in this occurrence, so that part of the proposed mechanism still needs reproduction.

No existing issue or pull request tracks this exact failure. The closest history is astral-sh/uv#13017, which added broad signal forwarding to fix the opposite failure reported in astral-sh/uv#12830, where externally delivered signals terminated uv without reaching its child. astral-sh/uv#11886 is adjacent because it explains the observed status 141: uv currently converts a signal-terminated child into a normal-looking 128+signal exit code, but that issue does not cover why the child received SIGPIPE.

## Draft response

Thanks for the report and reproducer. The current Unix child runner does register a SIGPIPE handler and unconditionally forwards any received SIGPIPE to the spawned command; that behavior was added in astral-sh/uv#13017 while fixing a different signal-forwarding problem. That supports treating the child termination as a uv bug.

We have not yet independently confirmed that the SIGPIPE originates from the reported HTTP/2 timeout/writev path. Could you capture a failing run with `uv -vv run --frozen -m modulename` and confirm whether `Received SIGPIPE, forwarding to child` appears immediately before the exit? The next implementation step is to reproduce this against current main and add regression coverage ensuring uv's own incidental SIGPIPE is not delivered to the child.

## Classification

This is a bug, not a duplicate. The current source unconditionally forwards SIGPIPE received by the uv parent to the spawned child. Although forwarding externally delivered signals is intentional, an incidental SIGPIPE arising from uv's own I/O is not a signal intended for the child; delivering it can terminate an otherwise healthy command. The attached reproduction and the claimed HTTP/2 timing have not been independently validated, but reproduction is not required to classify the source-confirmed correctness risk.

astral-sh/uv#12830 and astral-sh/uv#13017 are historical context rather than canonical trackers for this report. They address signals that should reach the child but did not. No open issue or pull request already tracks the newly reported failure mode, and no merged fix for this specific behavior was found.

## Related

- astral-sh/uv#13017 — Merged pull request, “Forward additional signals to the child process in `uv run`.” Its diff added the SIGPIPE listener and unconditional child forwarding that match the source-backed portion of this report. It fixed the opposite problem of signals not reaching children.
- astral-sh/uv#12830 — Closed bug, “Unexpected behavior when handling signals.” This was closed by astral-sh/uv#13017 and establishes the reason broad forwarding was added. Its symptom was orphaned children after externally delivered signals terminated uv, not uv-generated SIGPIPE terminating a child.
- astral-sh/uv#11886 — Open bug, “Handling of signal exit from subprocess is incorrect.” It tracks uv mapping signal termination to 128+signal without the shell's signal diagnostic, which accounts for status 141 after a SIGPIPE death. It does not track unintended SIGPIPE forwarding.

## Search evidence

Literal searches across open and closed issues and open, closed, and merged pull requests covered `SIGPIPE`, `EPIPE`, `writev`, “broken pipe,” “exit code 141,” “status 141,” “signal 13,” HTTP/2 timeouts, and network-triggered `uv run` termination. Conceptual searches covered signal forwarding and propagation, child processes, process groups, Unix/Tokio signal handling, and `uv run`. Fix-oriented searches inspected closed issues, merged pull requests, referenced discussions, and the history around astral-sh/uv#13017 and astral-sh/uv#12830.

astral-sh/uv#12244 was a plausible literal match because it discusses SIGPIPE and broken pipes, but it concerns shell-completion output writing to a closed stdout pipe and uv panicking; it does not involve network I/O or forwarding a signal to an `uv run` child. astral-sh/uv#3095 discusses replacing uv with the executed process on Unix, which would alter the general parent/child signal model, but it is a broad design enhancement rather than a tracker for this failure.

## Supporting source evidence

In `crates/uv/src/child.rs`, `run_to_completion` installs `handle_signal(SignalKind::pipe())`. Its select loop logs `Received SIGPIPE, forwarding to child` and invokes `signal::kill(child_pid, signal::Signal::SIGPIPE)` without checking whether the signal was externally addressed to uv or arose from uv's own I/O. When the child then terminates from signal 13, the same function maps that disposition to 128+13, yielding exit status 141. The astral-sh/uv#13017 diff shows that both SIGPIPE handling branches were introduced together.
