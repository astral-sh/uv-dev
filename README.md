# Windows trampoline can close its own child or job handle when started through the C runtime's spawn, so the child is lost

Issue: astral-sh/uv#22966

## Summary

The reporter describes intermittent child loss when a uv-created Windows virtual environment's `python.exe` is launched through `os.spawnv(os.P_WAIT, ...)` with non-inheritable pipe handles represented in the parent's CRT descriptor table. Reported symptoms are exit status 1 with `uv trampoline failed to get exit code of child process` and `os error 6`, or exit status 0 without the child's marker file. The base interpreter and CPython's own venv launcher reportedly succeed under the same conditions.

Reproduction outcome: **needs_more_information**. The report supplies a usable Windows fixture, but the available runner is Linux. A temporary Wine installation could not start even `cmd.exe`; no Windows child-loss experiment completed. Source inspection supports investigating the proposed handle collision, but does not confirm it. Native Windows execution evidence is still needed.

The closest history is astral-sh/uv#6699 and astral-sh/uv#6866, with merged cleanup fixes astral-sh/uv#6792 and astral-sh/uv#6955. Those fixes concern pointer decoding, descriptor layout, and warnings; they do not establish that this handle-collision case was previously fixed.

## Classification

Bug report concerning Windows trampoline process lifetime and inherited-handle cleanup; runtime confirmation remains pending. The reported behavior would violate the expected waiting and exit-status behavior of the launcher. The source closes non-sentinel CRT-table values after allocating child-process and kill-on-close job handles, making the proposed explanation plausible. No handle collision or particular closed handle was observed during this investigation.

The issue reports uv 0.13.5 and 0.13.6 as affected, but supplies no last known-good uv version and does not establish an upgrade regression. The corresponding release notes contain no fix for this collision. Historical cleanup fixes remain present in the inspected source.

## Reproduction

**Outcome: `needs_more_information` — Windows runtime validation is unavailable.** This is an execution-environment limitation, not a missing project configuration or a negative reproduction result.

### Reported environment and minimal trigger

- Windows 11 x86_64, build 10.0.26300; also GitHub Actions `windows-11-arm`.
- uv 0.13.6 (329bf9448, 2026-10-26), also 0.13.5.
- uv-managed CPython 3.14.9; also CPython 3.11.17 on ARM64.
- No project, workspace, dependency groups, or installed packages are required. The initial pytest-xdist failure is reduced to standalone CRT spawning.
- Create 500 anonymous pipes with `_winapi.CreatePipe(None, 0)`, yielding 1,000 non-inheritable handles, and wrap each with `msvcrt.open_osfhandle(handle, 0)`. Keep the resulting CRT descriptors open while making 200 `os.spawnv(os.P_WAIT, ...)` calls to a target interpreter. Each child executes code that creates a unique marker file.
- Expected: each call waits for the child; successful completion produces its marker file. Reported: intermittent missing markers with status 0 or 1, accompanied by descriptor-close warnings and sometimes the invalid-handle error. The reporter's five uv runs lost 0, 2, 8, 0, and 15 children; the two control launchers reportedly lost none. These counts have not been independently reproduced.

### Available environment and observations

The runner is Ubuntu 24.04.5 x86_64, Linux 6.17.0-1022-azure. The installed executable on PATH is `/opt/hostedtoolcache/uv/0.13.5/x86_64/uv`, reporting `uv 0.13.5 (x86_64-unknown-linux-gnu)`. System Python is CPython 3.12.3. Neither `msvcrt` nor `_winapi` is available to that Python, and neither Wine nor a native Windows runtime was initially installed.

All reproduction files, downloads, and caches were placed under `/tmp/uv-22966-7G8ITc`. The installed uv successfully prepared Windows artifacts with:

```sh
uv --no-config --cache-dir /tmp/uv-22966-7G8ITc/uv-cache python install --install-dir /tmp/uv-22966-7G8ITc/python --no-bin --no-registry cpython-3.14.9-windows-x86_64-none
uv --no-config --cache-dir /tmp/uv-22966-7G8ITc/uv-cache pip install --target /tmp/uv-22966-7G8ITc/windows-uv --python /usr/bin/python3 --python-platform x86_64-pc-windows-msvc --only-binary :all: --no-deps uv==0.13.5
```

Wine 9.0 Ubuntu packages were downloaded and extracted locally without system installation. A temporary wrapper confines its prefix, cache, and temporary files to the reproduction directory. Its packaged wineserver wrapper needed a local executable link, and its hard-coded `/run/user` socket-directory preference was redirected in the temporary runtime copies to permit fallback to temporary storage. Even after that setup, launching either Windows Python or the independent smoke check `./run-wine.sh cmd.exe /c ver` aborted with `free(): invalid pointer`, exit status 134. Windows uv and Python versions were therefore downloaded artifacts, not successfully executed runtimes. This setup failure provides no evidence for or against the reported trampoline behavior.

As a platform check, the installed Linux uv successfully ran:

```sh
uv --no-config --cache-dir /tmp/uv-22966-7G8ITc/uv-cache venv --python /usr/bin/python3 --no-python-downloads /tmp/uv-22966-7G8ITc/linux-venv
```

Its `bin/python` is a symlink to `/usr/bin/python3`, so executing that environment cannot exercise Windows trampoline cleanup. No Linux spawning result is being substituted for a Windows test.

An independently reconstructed fixture is saved at `/tmp/uv-22966-7G8ITc/reproduce.py`. It checks that the pipe handles are non-inheritable, retains the CRT descriptors through all spawns, records exit codes and missing markers on return, counts marker files again at the end, and closes descriptors afterward. It uses fresh marker directories and rejects whitespace-containing paths because CRT `spawnv` does not quote arguments. Its syntax was checked. Running it with Linux Python stops with the explicit message `Windows is required: this fixture needs msvcrt and _winapi.` No Windows child was launched by the fixture.

### Required follow-up

Run the fixture in a temporary directory without whitespace on native Windows, using a base interpreter matching one of the reported versions. For example, after placing `reproduce.py` in that directory and ensuring `python.exe` on PATH is the base interpreter, the following PowerShell commands construct the three comparisons. **These commands remain unexecuted on Windows:**

```powershell
$BasePython = (Get-Command python.exe).Source
uv --version
& $BasePython -VV
uv venv --python $BasePython uvvenv
& $BasePython -m venv --without-pip stdvenv
& $BasePython reproduce.py uvvenv\Scripts\python.exe 500 200
& $BasePython reproduce.py stdvenv\Scripts\python.exe 500 200
& $BasePython reproduce.py $BasePython 500 200
```

Keep uv's cache and Python installation directories inside the temporary reproduction directory. Repeat the uv invocation in fresh parent processes if the first result is clean, because the reported failure depends on handle allocation. Retain stderr, exit-code counts, missing-marker counts, interpreter paths, architecture, Windows build, and exact uv/Python versions. A missing marker immediately after return establishes a waiting/completion problem; confirming permanent termination or which live handle was closed requires additional observation or tracing. No extra project or dependency-group configuration is needed to attempt this fixture.

### Existing test coverage

Searched `crates/uv/tests/`, `crates/uv-client/tests/it/`, and `crates/uv-trampoline-builder/src/` for `spawnv`, `spawnve`, `lpReserved2`, `close_handles`, `GetHandleInformation`, and the reported cleanup/exit-code messages. No matching regression coverage was found. Nearby tests were read, including their setup and assertions:

- `crates/uv-trampoline-builder/src/lib.rs::test::console_python_launcher` creates a Python launcher and invokes it through Rust `Command`, checks successful output, checks launcher metadata, signs the executable, and invokes it again. It does not use CRT spawn or populate a descriptor table with non-inheritable handles.
- `crates/uv-trampoline-builder/src/lib.rs::test::console_script_launcher` checks console output, argument forwarding, launcher metadata, and execution after signing. It likewise lacks the reported descriptor-table setup.
- `crates/uv/tests/python/venv.rs::windows_shims` creates a batch shim selecting Python 3.9 over Python 3.10, then checks interpreter selection and successful venv creation. It does not exercise child waiting or handle cleanup.

The builder's test module is gated by `cfg(all(test, windows))`. The venv module is gated by `test-python`, and `windows_shims` additionally by `cfg(windows)`. These tests were inspected, not executed. No repository tests or builds were run or modified.

## Supporting source evidence

- `crates/uv-trampoline/src/bounce.rs:419` creates the child process; the job is then created and assigned before `close_handles` at line 447.
- At line 335, cleanup reads the CRT descriptor count and handle array. The loop at line 342 skips the first three descriptors and invalid sentinel values, then calls `CloseHandle` on remaining values. It does not first record whether those numeric values represented inherited handles before child/job creation.
- At line 465, the wait result is discarded. A subsequent `GetExitCodeProcess` failure produces the reported message. `crates/uv-windows/src/job.rs:127` sets `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
- The local `bounce.rs` blob is `b81de8f7a478767be2ac2f5b3f007bf2bfdba248`. Existing issue-context research records the same GitHub blob ID for tags 0.13.5 and 0.13.6.
- The historical diffs for astral-sh/uv#6792 and astral-sh/uv#6955 correct pointer interpretation, array layout, standard-descriptor skipping, and sentinel handling. They do not record which handle values were inherited.
- pypa/pip#10444 explains the rationale for cleaning up handles after child creation so that the child can inherit them. It does not confirm the reported collision in distlib, which the reporter explicitly did not test.

This ordering is consistent with the proposed explanation. It does not independently establish a collision, its frequency, the silently successful exit, or a confirmed root cause. The proposed approach of recording inherited handles before allocating new handles, then closing them after spawning, still needs validation; checking validity only after spawning would not establish ownership.

## Related

- astral-sh/uv#6699 (issue, closed): Windows stubs exited abnormally when launched through BusyBox's Microsoft CRT `spawnve`. The confirmed failures concerned access violations and pointer-layout problems, fixed in 2024. This is close historical context, not a confirmed duplicate of live child/job handle closure.
- astral-sh/uv#6792 (pull request, merged August 30, 2024): Fixed astral-sh/uv#6699 by treating table entries as HANDLE values instead of dereferencing them as pointers, and skipping invalid sentinels. It did not establish handle inheritance before new handle allocation.
- astral-sh/uv#6955 (pull request, merged; released in uv 0.4.5): Corrected CRT-table byte offsets, skipped the first three descriptors, and excluded the -2 sentinel. It addressed warnings in astral-sh/uv#6699 and astral-sh/uv#6866. Those corrections remain in the current source.
- astral-sh/uv#6866 (issue, closed): CMake stub crashes when invoked by VS Code, followed by descriptor-close warnings. The reporter confirmed uv 0.4.5 resolved the remaining warning. The thread does not confirm the current collision mechanism.

## Other candidates distinguished

- astral-sh/uv#20100 concerns a disappearing GUI under `uv run`, while direct trampoline invocation succeeds. CRT descriptor-table involvement was not established.
- astral-sh/uv#18168, astral-sh/uv#18170, open astral-sh/uv#18171, and merged astral-sh/uv#18291 concern job-assignment failures or supervisor timing races. Making assignment failure non-fatal does not address later handle cleanup. astral-sh/uv#18171 concerns `uvx` and `uv run` rather than this trampoline loop.
- astral-sh/uv#17374 restricts standard-input/output cleanup to pipes; astral-sh/uv#17174 concerns debugger history and a separate CPython issue. The CRT descriptor loop is unchanged by that fix.
- astral-sh/uv#6464 is a historical access-violation report confirmed fixed in uv 0.4.1. astral-sh/uv#2445 concerns Python 3.7 interpreting an executable as source; its handle-related strings were extracted from a binary rather than observed errors.

The related-issue research covered open and closed issues and pull requests, literal error/Win32 API searches, conceptual searches for child loss and handle inheritance, file history, changelogs, and linked discussions. None of those references replaces a successful runtime reproduction of astral-sh/uv#22966.
