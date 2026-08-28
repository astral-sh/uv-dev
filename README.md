# uvx crashes with SIGABRT / abort() on macOS when launched non-interactively via subprocess stdio piping

Issue: astral-sh/uv#21337

Classification: bug

## Summary

The reporter says `uvx mcp-server-git` works in an interactive shell but uv 0.12.7 aborts with SIGABRT on Apple Silicon macOS 26.6.2 when opencode launches it without a PTY and pipes stdin/stdout for JSON-RPC. The report expects `uvx` to run the tool normally with inherited piped standard streams and without terminal progress rendering.

The reporter subsequently supplied the complete sanitized Apple crash report. It confirms that the native ARM64 Homebrew `uv` process was directly parented by opencode, with iTerm2 recorded as the responsible process and System Integrity Protection enabled. The process launched at 12:21:43.4464 and the crash was captured at 12:21:43.4575, so the abort occurred roughly 11 ms after launch. The main thread was waiting in `_pthread_join`, `main2` called `abort`, and a Tokio runtime worker was waiting on a condition variable. All uv frames remain unsymbolicated, however, and the report contains neither Rust stderr nor a Rust panic message. The uv image UUID and offsets could be used to symbolicate the exact Homebrew binary.

A controlled subprocess reproduction on the available Linux x86_64 runner did not abort. Using the installed uvx 0.12.7, a local console-script fixture received a JSON-RPC-shaped line through piped stdin, echoed it through piped stdout, and exited successfully while stderr was also piped. Repeating with `UV_NO_PROGRESS=1` also succeeded. This demonstrates that piped standard streams alone do not cause the reported failure on Linux, but it does not test the reported macOS ARM64 path, the published `mcp-server-git` process, or any restrictions imposed by opencode.

No existing issue or pull request matches the full combination of `uvx` tool execution, connected piped stdin/stdout, no PTY, immediate SIGABRT, and this uv release. The closest reports establish three adjacent but materially different behaviors: noninteractive progress is intentionally hidden; an open broken-pipe panic occurs only after a shell-completion reader disconnects; and an older macOS external-host panic had a specific SystemConfiguration sandbox-denial signature that was fixed upstream.

Despite the complete Apple crash report, the issue still does not include the Rust panic message or stderr, a symbolicated Rust backtrace, a standalone spawning script with the actual spawn options and environment, the child exit status as observed by the parent, or confirmation that opencode is not applying a macOS sandbox. Those details, plus a run on the affected macOS host, are needed to identify the abort site and test whether this is related to the historical sandbox failure or to stdio handling.

## Reproduction

Outcome: `needs_more_information`.

The new macOS crash report strengthens the original observation but is not a standalone reproduction. It establishes an immediate abort in the uv process itself while opencode is its parent, rather than merely reporting a failure from the launched Python tool. Because the uv frames are unsymbolicated and there is no Rust stderr, it does not identify which uv or dependency code path called `abort` or establish that piped stdio was the trigger.

The available environment is Linux x86_64, not the reported macOS 26.6.2 ARM64 environment:

```console
$ uvx --version
uvx 0.12.7 (x86_64-unknown-linux-gnu)
$ python3 --version
Python 3.12.3
```

All fixture files, uv caches, tool state, and Python-install state were placed under `$RUNNER_TEMP/uv-issue-21337.MarkCM`. The fixture was a local package exposing `uv-stdio-probe`; its console script reads newline-delimited JSON from stdin and writes the decoded value as JSON to stdout. A standalone Python parent used the following subprocess shape, which reconstructs the reported non-PTY stdio arrangement without depending on opencode or executing the reported third-party package:

```python
process = subprocess.Popen(
    ["uvx", "--from", local_package, "uv-stdio-probe"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    env=environment,
)
stdout, stderr = process.communicate(
    json.dumps({"jsonrpc": "2.0", "id": 1}) + "\n",
    timeout=30,
)
```

The environment selected the system Python and redirected uv state into that temporary directory with `UV_CACHE_DIR`, `UV_TOOL_DIR`, and `UV_PYTHON_INSTALL_DIR`; `RUST_BACKTRACE=full` was set. The initial run produced:

```text
returncode: 0
stdout: {"received": {"jsonrpc": "2.0", "id": 1}}
stderr: fixture build/install diagnostics
```

The warm-cache run with `UV_NO_PROGRESS=1` likewise returned 0 with the same stdout and empty stderr. A third run piped only stdin and stdout while inheriting stderr, matching the streams explicitly named in the report; it also returned 0 with the same stdout. No panic or signal termination occurred.

Existing integration coverage is adjacent but not complete. `crates/uv/tests/tool/tool_run.rs`, including `tool_run_args` and `tool_run_without_output`, verifies that tool processes execute successfully while stdout and stderr are captured and checks noninteractive installer output. After reading their setup and snapshots, neither test provides a connected piped stdin to a long-running tool. No tool-run test matching `uvx` plus piped stdin/stdout/stderr was found under `crates/uv/tests/` or `crates/uv-client/tests/it/`.

The Linux result is only a simplified control, so it is not evidence that the configuration-dependent macOS report is `not_reproducible`. A meaningful targeted reproduction still needs the exact standalone Node or Python spawn program from the affected macOS ARM64 host, its full child environment and options, complete stderr with `RUST_BACKTRACE=full`, the exit status or terminating signal observed by the parent, and whether opencode applies sandbox restrictions. Results from that same program both outside opencode and with `UV_NO_PROGRESS=1` would isolate the host and progress-rendering variables. If stderr cannot be captured before the immediate abort, symbolication using the reported uv image UUID and offsets is the next useful diagnostic path.

## Draft response

Thanks for the report. Launching a tool with piped standard streams and no PTY should not abort, so this is a bug if the abort is coming from uv. A local JSON-line console-script fixture launched through uvx 0.12.7 with stdin, stdout, and stderr piped succeeds on Linux, including with `UV_NO_PROGRESS=1`; therefore, piped stdio by itself does not reproduce the abort in the available environment.

The macOS crash excerpt only shows the final `abort()` frames; it does not include the Rust panic message or the frame that identifies the failing component. Could you provide:

1. A minimal Node or Python script with the exact spawn options and child environment that reproduces this by spawning `uvx` directly on the affected Mac, without opencode.
2. The complete stderr output with `RUST_BACKTRACE=full` set for the child process, plus its exit status.
3. The result of the same reproduction with `UV_NO_PROGRESS=1`.
4. Whether opencode applies a macOS sandbox or other execution restrictions to child processes, and whether the standalone script behaves differently inside and outside those restrictions.

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

Source inspection and the controlled Linux run support requesting a macOS reproduction rather than accepting the proposed TTY cause as established: `uvx` constructs the tool command with inherited standard streams, the Unix child supervisor explicitly checks whether stdin is a terminal to choose signal-forwarding behavior, and the integration tool-run tests routinely capture process output. However, those tests do not cover connected piped stdin for tool execution. The Apple crash report now confirms the immediate native uv abort and provides offsets for symbolication, but the missing Rust panic message, exact standalone spawn configuration, and sandbox status remain the decisive evidence gaps.
