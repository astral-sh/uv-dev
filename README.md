# uvx crashes with SIGABRT / abort() on macOS when launched non-interactively via subprocess stdio piping

Issue: astral-sh/uv#21337

Classification: bug

## Summary

The reporter says `uvx mcp-server-git` works in an interactive shell but uv 0.12.7 aborts with SIGABRT on Apple Silicon macOS 26.6.2 when opencode launches it without a PTY and pipes stdin/stdout for JSON-RPC. The report expects `uvx` to run the tool normally with inherited piped standard streams and without terminal progress rendering.

No existing issue or pull request matches the full combination of `uvx` tool execution, connected piped stdin/stdout, no PTY, immediate SIGABRT, and this uv release. The closest reports establish three adjacent but materially different behaviors: noninteractive progress is intentionally hidden; an open broken-pipe panic occurs only after a shell-completion reader disconnects; and an older macOS external-host panic had a specific SystemConfiguration sandbox-denial signature that was fixed upstream.

The report does not include the Rust panic message, a Rust backtrace, a standalone spawning script, the child exit status, or confirmation that opencode is not applying a macOS sandbox. Those details are needed to distinguish an uv panic from a failure in the launched tool and to test whether this is related to the historical sandbox failure or to stdio handling.

## Draft response

Thanks for the report. Launching a tool with piped standard streams and no PTY should not abort, so this is a bug if the abort is coming from uv.

The macOS crash excerpt only shows the final `abort()` frames; it does not include the Rust panic message or the frame that identifies the failing component. Could you provide:

1. A minimal Node or Python script that reproduces this by spawning `uvx` directly, without opencode.
2. The complete stderr output with `RUST_BACKTRACE=full` set for the child process, plus its exit status.
3. The result of the same reproduction with `UV_NO_PROGRESS=1`.
4. Whether opencode applies a macOS sandbox or other execution restrictions to child processes.

This does not currently have the signature from the previously fixed sandbox panic in astral-sh/uv#16916, and astral-sh/uv#12244 concerns a different case where a shell-completion pipe reader has already closed. The requested output will let us determine whether either mechanism is relevant without assuming that the lack of a PTY is the cause.

## Classification

This is a `bug`. A native abort instead of launching the requested tool is incorrect behavior in a normal subprocess configuration. The repository's tool-run implementation is designed to inherit stdin, stdout, and stderr, and it already uses terminal detection for signal-forwarding behavior. Progress rendering targets stderr through indicatif, whose nonterminal behavior is discussed in astral-sh/uv#11121. Therefore, piped stdio is not itself documented as unsupported.

The issue is not a duplicate on the current evidence. It lacks the diagnostic frame needed to connect it to a known panic, and no existing report has the same command, trigger, failure, and version. It is also not yet established as a regression: the closest historical macOS sandbox panic was tied to `system-configuration` 0.6.1 and `hyper-util` before 0.1.20, while the current checkout uses `system-configuration` 0.7.0 and `hyper-util` 0.1.20.

## Related

- astral-sh/uv#11121 — **[feat]: Request for more output in stdout/stderr for non-interactive process** (open). This uses Node `spawn` and Python `subprocess` without a terminal, so the execution context is close. It reports suppressed download progress rather than a crash. A maintainer explains that interactive progress bars intentionally do not render in noninteractive mode, making it evidence against treating progress suppression and SIGABRT as the same problem.
- astral-sh/uv#12244 — **uv panics when resizing terminal window** (open). This is the closest pipe-related panic: macOS reports a `main2` panic followed by the outer Tokio-executor panic. Its complete diagnostic identifies `clap_complete` writing fish shell completions after the reader closes the pipe with `BrokenPipe`. The new issue instead describes a connected IPC pipe while running a tool and supplies no `BrokenPipe` message. Open pull request astral-sh/uv#18584 only changes shell-completion generation, so it does not track this report.
- astral-sh/uv#16916 — **uv init fails when called from Claude Code's sandbox on macOS** (closed). This is the closest historical external-host/macOS panic before a target process starts. It was confirmed to panic in `system-configuration` when a sandbox denied access to macOS SystemConfiguration, and maintainers tracked an upstream fix that shipped with newer dependencies. The new issue uses a later uv release and neither mentions a sandbox nor includes the identifying `Attempted to create a NULL object` panic, so it is a lead to check rather than a duplicate.

## Supporting evidence and search scope

Searches covered open and closed issues and open, closed, and merged pull requests. Literal searches used `SIGABRT`, `Abort trap`, `uvx`, `stdin`, `stdout`, `pipe`, `subprocess`, `spawn`, `MCP`, `PTY`, `posix_spawn`, `Tokio worker`, `tokio-runtime-worker`, and `main2`. Conceptual searches covered noninteractive execution, redirected streams, terminal detection, progress output, broken pipes, child-process signal forwarding, macOS ARM64 panics, and sandboxed external hosts. Fix-oriented review included closed macOS panic reports, their linked upstream fixes, current dependency versions, and the open shell-completion BrokenPipe fix.

No direct predecessor was found. astral-sh/uv#18584 was inspected and ruled out because it only buffers shell-completion output and handles a disconnected completion consumer. astral-sh/uv#11817 and astral-sh/uv#12692 were also ruled out: they concern terminating process trees, not a child launch that aborts immediately. Generic Tokio-worker crashes and macOS certificate/network panics had different commands, exact errors, and confirmed mechanisms.

Source inspection supports requesting a minimal reproduction rather than accepting the proposed TTY cause as established: `uvx` constructs the tool command with inherited standard streams, the Unix child supervisor explicitly checks whether stdin is a terminal to choose signal-forwarding behavior, and the integration tool-run tests routinely capture process output. The missing first panic message and standalone reproduction remain the decisive evidence gaps.
