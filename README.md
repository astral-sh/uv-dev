# Interpreter discovery aborts on a PATH candidate that fails to execute (pyenv shim, exit 127) instead of skipping it

Issue: astral-sh/uv#21047

Classification: bug

## Summary

The report says `uv run --no-project --python 3.13` fails when the first Python-like executable on `PATH` is a pyenv-style shim that runs but exits 127. The reporter expects uv to discard that candidate and select a later working Python 3.13. The supplied standalone reproduction creates the failing `python` shim and shows the error, but it does not create or identify the later Python 3.13 candidate.

Repository source distinguishes failures that prevent spawning an executable from query processes that complete unsuccessfully. A nonzero query status is non-critical: discovery records the first error, continues scanning, and returns a matching installation if one is found. It resurfaces the recorded error only after finding no usable interpreter. Therefore the reported result would be a bug if a later runnable interpreter really satisfies the request, but the claimed reason that exit 127 immediately aborts scanning is not confirmed. A complete verbose trace and the executable paths are needed to identify why the later interpreter is absent or rejected.

No existing issue exactly tracks that selection failure. The closest historical report is astral-sh/uv#13402, which contains the same pyenv exit-127 diagnostic, while astral-sh/uv#10716 introduced the policy that explains when the retained error is surfaced. astral-sh/uv#15667 and astral-sh/uv#15315 cover adjacent execution-failure paths.

## Draft response

Thanks for the reproduction. A query that exits nonzero is currently treated as a non-critical discovery error: uv should continue scanning and only surface the first such error if it finds no interpreter satisfying the request. That behavior was introduced in astral-sh/uv#10716, and astral-sh/uv#13402 shows the same pyenv exit-127 diagnostic when no matching runnable interpreter was found.

Could you provide the complete output from the failing command with `-vv`, along with `type -a python3.13 python3 python` and the successful `--version` output for the later Python 3.13 executable? The standalone reproduction creates the failing shim but does not show the later 3.13 candidate, so those details are needed to determine why discovery is not considering or accepting it.

This is distinct from astral-sh/uv#15667, where spawning a foreign-architecture executable fails before it can return a status.

## Classification

Classify as a bug. Failing instead of selecting a later `PATH` interpreter that satisfies the explicit Python 3.13 request is incorrect observable behavior. Current source says a status-code query failure should be skipped while discovery continues, so this classification depends on the report's assertion that a later runnable matching interpreter exists; the exact mechanism remains unconfirmed pending a full trace.

This is not a duplicate of astral-sh/uv#13402. That historical report had the same pyenv exit-127 error but did not show a later runnable interpreter satisfying the request, and its `uv python pin` reproduction was reported no longer reproducible in uv 0.7.13. It is not a duplicate of astral-sh/uv#15667 because that issue concerns a critical spawn/I/O failure during exhaustive `uv python list`, rather than a process that exits with status 127 during selection.

## Related

- astral-sh/uv#13402 — closed issue, “Collision with pyenv when `uv python pin <version>`.” This is the closest historical report. It includes a pyenv shim exiting 127 with the same “first executable in the search path” diagnostic. A maintainer explained that uv already skips the error during discovery and resurfaces it only if no appropriate interpreter is found. The report later became unreproducible for `uv python pin` in 0.7.13.

- astral-sh/uv#10716 — merged pull request, “Show non-critical Python discovery errors if no other interpreter is found.” It deliberately introduced retention of the first non-critical query error so uv can show that error after discovery finds no usable interpreter. This directly explains the final diagnostic in the new report.

- astral-sh/uv#15667 — open issue, “`uv python list` errors with `Exec format error` on non-native installation.” It also reports discovery aborting on an unexecutable PATH candidate, but the process cannot be spawned and the command exhaustively lists interpreters. The new report's shim does run and returns status 127 during selection.

- astral-sh/uv#15315 — merged pull request, “Skip interpreters that are not found on query.” It fixed a related pyenv-win case in which an existing discovered shim could not be queried. Its not-found spawn failure is different from a shim process returning status 127, but it establishes a precedent for skipping unusable discovered candidates.

## Search and supporting evidence

Literal searches covered the exact “Failed to inspect Python interpreter from first executable in the search path” and “Querying Python … failed with exit status” diagnostics, exit 127, and pyenv “command not found” shims. Conceptual searches covered broken or unusable PATH candidates, non-critical discovery errors, continuing to later interpreters, version-specific executable lookup, and download fallback. Searches included open and closed issues and open, closed, and merged pull requests.

The strongest candidates and their comments and linked changes were inspected: astral-sh/uv#13402, astral-sh/uv#15667, astral-sh/uv#10898, astral-sh/uv#12155, astral-sh/uv#4709, astral-sh/uv#10716, astral-sh/uv#10908, astral-sh/uv#15315, and astral-sh/uv#5148.

astral-sh/uv#4709 and astral-sh/uv#5148 were ruled out as the canonical match. They address finding version-named executables after runnable default interpreters have the wrong version, not handling a query failure; current source still scans multiple names in every PATH directory. astral-sh/uv#10898 and astral-sh/uv#10908 concern allowing a managed-Python download after non-critical discovery errors and are less direct because astral-sh/uv#21047 explicitly disables downloads.

The current discovery implementation marks interpreter status-code failures as non-critical, logs that it is skipping the bad interpreter, and continues iterating. It returns the first successful matching installation immediately. Only after all candidates fail does it return the first recorded non-critical error. An existing integration test also expects the first broken PATH candidate's error to be surfaced when no matching usable interpreter is found, but not when the broken candidate is encountered after a usable interpreter.
