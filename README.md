# Windows trampoline can close its own child or job handle when started through the C runtime's spawn, so the child is lost

Issue: astral-sh/uv#22966

Classification: bug

## Summary

The closest history is astral-sh/uv#6699 and astral-sh/uv#6866, with merged cleanup fixes astral-sh/uv#6792 and astral-sh/uv#6955. They addressed pointer decoding and descriptor warnings, leaving the reported handle-collision case unaddressed.

## Draft response

The source confirms the ordering you describe: close_handles runs after creating the child and job handles, and it filters sentinel values without checking which handles were inherited. That leaves the cleanup vulnerable to the collision you describe. The earlier fixes in astral-sh/uv#6792 and astral-sh/uv#6955 address different problems in the same code.

The next step is to validate your reproduction on Windows and add regression coverage for both the invalid-handle error and silent child loss. A fix needs to distinguish inherited handles from newly allocated ones while retaining cleanup after the child inherits its descriptors.

## Classification

The current source closes non-sentinel lpReserved2 values after allocating child-process and kill-on-close job handles, without establishing that those values were inherited. This leaves live trampoline-owned handles exposed to erroneous closure and supports a correctness bug. No existing discussion was found that tracks this specific problem. The historical pointer and standard-descriptor fixes remain present; there is no evidence that a previously fixed handle-collision bug regressed. The Windows reproduction was not run.

## Related

- astral-sh/uv#6699 (issue, closed): Stubs on Windows exit abnormally, includes succinct repro with busybox, related to some wider issue. Closest historical report: BusyBox used the Microsoft C runtime's spawnve, and trampoline cleanup encountered invalid lpReserved2 entries and descriptor-close warnings. The confirmed failures were access violations and later pointer-layout problems, fixed in 2024; the thread does not establish the newly reported collision with live child or job handles.
- astral-sh/uv#6792 (pull request, merged): fix: adjust trampoline close_handles invalid to be safer. Fixed astral-sh/uv#6699 by treating table entries as HANDLE values instead of dereferencing them as pointers and skipping invalid sentinel values. Merged August 30, 2024. It did not establish whether handles were inherited before creating new handles, so it does not fix the reported collision.
- astral-sh/uv#6955 (pull request, merged): fix: adjust close_handles pointer offsets to match distlib cleanup_fds. Follow-up cleanup fix, released in uv 0.4.5: corrected lpReserved2 byte offsets, skipped the first three descriptors, and excluded the -2 sentinel. It addressed warnings in astral-sh/uv#6699 and astral-sh/uv#6866. Those corrections remain in current source, but non-inherited, non-sentinel values are still closed after child and job creation.
- astral-sh/uv#6866 (issue, closed): CMake stub crashes when executed by VS Code. Reported the same descriptor-close warning after an earlier trampoline crash was fixed. The reporter confirmed the remaining warning was fixed in uv 0.4.5 by the pointer-layout cleanup. This is historical evidence for the cleanup code, not a confirmed instance of the present C-runtime handle-collision problem.

## Report and trigger

The reporter runs a uv-created virtual environment's Windows `python.exe` trampoline through the C runtime via `os.spawnv(os.P_WAIT, ...)`. The supplied reproduction populates the parent's CRT descriptor table with non-inheritable pipe handles wrapped by `msvcrt.open_osfhandle(handle, 0)`, then launches children that should create marker files.

Reported outcomes are intermittent exit status 1 with `uv trampoline failed to get exit code of child process` and `os error 6`, or status 0 without the child's marker file. Descriptor-close warnings occur on nearly every supplied descriptor. The reporter observed no missing children with CPython's own venv launcher or the base interpreter under the same conditions.

Affected versions reported: uv 0.13.5 and 0.13.6; CPython 3.11.17 and 3.14.9; Windows 11 x86_64 and GitHub Actions Windows ARM64. The initial failure occurred in a pytest-xdist worker, but the supplied reproduction removes that dependency. This is a request to correct launcher behavior, not a request for a new spawning API.

## Supporting evidence

- In `crates/uv-trampoline/src/bounce.rs:419`, `bounce` creates the child process, then creates and assigns the job, before calling `close_handles` at line 447.
- At `crates/uv-trampoline/src/bounce.rs:335`, cleanup reads the CRT descriptor count and handle array. The loop at line 342 skips the first three descriptors and sentinel invalid values, then calls `CloseHandle` on every remaining value. It does not verify that each value represented an inherited handle before new handles were allocated.
- At `crates/uv-trampoline/src/bounce.rs:465`, the wait result is discarded; `GetExitCodeProcess` failure produces the exact reported error. At `crates/uv-windows/src/job.rs:127`, the job is configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
- GitHub's file metadata for tags 0.13.5 and 0.13.6 and the local file all have Git blob ID `b81de8f7a478767be2ac2f5b3f007bf2bfdba248`. The reported cleanup code is therefore present in both releases and this checkout.
- The diffs for astral-sh/uv#6792 and astral-sh/uv#6955 confirm that the historical fixes corrected pointer interpretation, array layout, standard-descriptor skipping, and sentinel handling. They did not record which handle values were inherited. The 0.4.5 changelog includes astral-sh/uv#6955; reporter comments in astral-sh/uv#6866 confirm that release fixed the earlier warnings.
- The discussion in pypa/pip#10444 explains why standard-handle cleanup must happen after child creation to preserve inheritance. It does not confirm the newly reported live-handle collision in distlib, and the reporter explicitly did not test distlib.

These findings establish the vulnerable cleanup ordering and missing ownership distinction. The reproduction was not executed on Windows, so the specific process-handle and job-handle collisions, their frequency, and the reported zero exit status remain runtime observations supplied by the reporter.

## Candidates distinguished

astral-sh/uv#20100 reports a disappearing GUI when invoked through `uv run`, while direct trampoline invocation succeeds; its comments do not establish CRT descriptor-table involvement. It is not a confirmed duplicate.

astral-sh/uv#18168, astral-sh/uv#18170, open astral-sh/uv#18171, and merged astral-sh/uv#18291 concern job-assignment failures or supervisor timing races. The chosen fix makes assignment failure non-fatal; it does not prevent subsequent descriptor cleanup from closing a live handle. astral-sh/uv#18171 targets `uvx` and `uv run`, not the trampoline cleanup loop.

astral-sh/uv#17374 restricts standard-input/output cleanup to pipes. Its originating discussion, astral-sh/uv#17174, concerns debugger history and a separate CPython issue. That change leaves the CRT descriptor loop untouched.

astral-sh/uv#6464 is another historical access-violation report confirmed fixed in uv 0.4.1. astral-sh/uv#2445 concerns Python 3.7 interpreting an executable as source; its handle-related matches were strings extracted from the binary, not observed handle errors.

## Recommended next step

Validate the supplied reproduction on Windows x86_64 and ARM64, recording child completion as well as exit status. Build focused regression coverage for the process-handle and job-handle cases and verify that genuinely inherited descriptors still reach the child.

Review a cleanup design that identifies eligible inherited handles before allocating trampoline-owned handles, while closing them only after the child has inherited what it needs. The suggested `GetHandleInformation` approach is a proposal to evaluate, not a validated fix. Checking handle validity only after spawning would not distinguish an inherited handle from a newly allocated process or job handle.

No matching `spawnv`, `lpReserved2`, `close_handles`, or `GetHandleInformation` coverage was found in `crates/uv/tests` or `crates/uv-trampoline-builder/src`. A downstream implementation should check existing test structure and feature gates, add explicit scenarios without snapshot-sharing loops, rebuild the bundled trampolines, and run the relevant Windows checks. No builds or tests were run for this handoff.

## Search scope

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered the exit-code error, 'os error 6', descriptor-close warning, os.spawnv, spawnve, lpReserved2, CloseHandle, GetExitCodeProcess, and close_handles. Separate conceptual searches covered trampoline child loss, early returns, termination, crashes, Windows stubs, job handles, and non-inheritable descriptors, without package or version restrictions; area:windows searches supplied additional context. PR keyword searches missed known fixes, so coverage was supplemented with paginated PR titles/bodies across all states, file commit history, changelogs, and direct inspection of candidate discussions and diffs. Followed astral-sh/uv#6699 through astral-sh/uv#6792 and astral-sh/uv#6955, including astral-sh/uv#6464 and astral-sh/uv#6866. Ruled out astral-sh/uv#20100 (GUI lifetime), astral-sh/uv#18170, astral-sh/uv#18171, and astral-sh/uv#18291 (job-assignment races), and astral-sh/uv#17374 (standard console handles). The reporter's pypa/pip#10444 link establishes post-spawn cleanup rationale, not this collision.
