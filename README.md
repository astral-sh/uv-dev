# uv aborts (SIGABRT, "failed to create UnixStream: Function not implemented") in the i686 manylinux container on GitHub's ubuntu-26.04 runner

Issue: astral-sh/uv#23147

Classification: bug

## Summary

No closely related issue or pull request was found. Existing SIGABRT, UnixStream, ENOSYS, and i686 reports concern different operations or mechanisms; none establishes the cause of this container-specific abort.

The reporter observes `/usr/local/bin/uv pip install` terminating with SIGABRT while `python -m build --installer=uv` installs isolated build requirements under cibuildwheel. The diagnostic is:

```text
failed to create UnixStream: Os { code: 38, kind: Unsupported, message: "Function not implemented" }
```

The failing environment is the 32-bit `quay.io/pypa/manylinux_2_28_i686` container on a GitHub-hosted ubuntu-26.04 x86_64 runner, with Python 3.10.22. The reporter says the x86_64 container succeeds on that runner and the same i686 container image digest succeeded on ubuntu-24.04 on 2026-10-29. The first reported failure is 2026-11-07. These comparisons are reported observations; they have not been independently reproduced.

| Component | Last successful run | Failing run |
| --- | --- | --- |
| Runner image | ubuntu-24.04 20261025.353.1 | ubuntu-26.04 20261101.186.1 |
| Docker server | 28.0.4 | 29.4.2 |
| Kernel | 7.0.0-1017-azure | 7.0.0-1019-azure |
| Host setup-uv version | 0.13.6 | 0.13.7 |
| uv version inside the container | Not captured | Not captured |

The shared container digest is `sha256:541324ddb178a1611c25485a8fcce73c227131f273a510006b9fb0bfa0672d27`. Holding the container image constant does not hold uv constant, because cibuildwheel installs uv separately. The listed host versions must not be used to identify the crashing executable.

The requested requirements were `cython>=3.2.9`, `setuptools-scm>=8`, `setuptools>=84`, `wheel`, and `ziglang>=0.17.1`. There is no evidence identifying any one requirement as the trigger.

## Reproduction

**Outcome: needs_more_information.** The reported SIGABRT/ENOSYS was not observed in the available environment. Successful x86_64 controls cannot determine whether the reported Ubuntu 26.04/i686 container failure reproduces. The crashing container's uv version is missing, and the affected runner/container environment is unavailable here.

### Available environment and limits

- Ubuntu 24.04.5 LTS, x86_64, kernel `7.0.0-1017-azure`.
- Installed executable on PATH: `/opt/hostedtoolcache/uv/0.13.6/x86_64/uv`, reporting `uv 0.13.6 (x86_64-unknown-linux-gnu)`.
- Managed CPython 3.10.22 for Linux x86_64, downloaded with this uv into the reproduction directory. It matches the reported Python version but not the reported i686 architecture or manylinux interpreter build.
- Docker client 28.0.4 is present, but `docker version` fails to connect to `/var/run/docker.sock` with permission denied. The Docker server version could not be inspected. No Docker container, default/unconfined seccomp comparison, or Ubuntu 26.04 execution was possible.
- All reproduction environments, downloads, caches, logs, and the syscall trace are under `/tmp/uv-23147-dpkonoeo`. Subprocesses used a clean environment, temporary configuration/home directories, and `UV_NO_CONFIG=1`; they did not inherit project configuration or registry credentials. No checkout files or existing user environments were modified.

### Commands and observed results

The following summarizes the commands run from the temporary directory. `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR`, `UV_PYTHON_BIN_DIR`, and `TMPDIR` were set to subdirectories of that directory; progress and configuration discovery were disabled.

```sh
uv python install 3.10.22
uv venv --python 3.10.22 /tmp/uv-23147-dpkonoeo/wheel-env
strace -f -e trace=socketpair -o /tmp/uv-23147-dpkonoeo/socketpair.trace \
  uv pip install --python /tmp/uv-23147-dpkonoeo/wheel-env/bin/python wheel
uv venv --python 3.10.22 /tmp/uv-23147-dpkonoeo/full-env
uv pip install 'ziglang>=0.17.1' 'setuptools-scm>=8' 'cython>=3.2.9' \
  'setuptools>=84' wheel --python /tmp/uv-23147-dpkonoeo/full-env/bin/python
```

Every command exited 0. The minimal install installed `wheel==0.48.0` and `packaging==26.3`. Its trace recorded:

```text
socketpair(AF_UNIX, SOCK_STREAM|SOCK_CLOEXEC|SOCK_NONBLOCK, 0, [6, 7]) = 0
```

The full requirement set installed nine packages: `cython==3.3.1`, `packaging==26.3`, `setuptools==84.2.1`, `setuptools-scm==10.3.4`, `tomli==2.5.0`, `typing-extensions==4.16.0`, `vcs-versioning==2.6.0`, `wheel==0.48.0`, and `ziglang==0.17.1`. Neither control printed the UnixStream diagnostic or terminated with a signal. The full install exercises the exact reported requirement bounds and explicit interpreter selection, but does not reproduce the outer cibuildwheel/build frontend or the failing platform.

### Existing test coverage

Searches of Rust tests under `crates/uv/tests/` and `crates/uv-client/tests/it/` found no targeted coverage for this UnixStream/ENOSYS failure, socketpair filtering, or the reported i686 container. The setup and assertions of `crates/uv/tests/pip_install/pip_install.rs::install_package` were read: it creates a Python 3.12 test environment, installs Flask with `--strict`, snapshots a successful seven-package install, and verifies `import flask`. The module is gated by both `test-python` and `test-pypi` in `crates/uv/tests/pip_install/main.rs`. This covers ordinary pip installation, not 32-bit runtime initialization under this Docker/kernel combination. The platform policy also states that the full test suite is not run on Tier 2 targets. No existing test was run or modified during this reproduction attempt.

### Information needed to resolve the outcome

1. The version and architecture of `/usr/local/bin/uv` inside the failing container, and preferably the corresponding values from the successful run. Host setup-uv versions do not provide this information.
2. A run of the minimal wheel install on the affected Ubuntu 26.04 runner using the pinned container digest and pinned container uv, with verbose output. Compare the same inputs on Ubuntu 24.04 to isolate the runner change.
3. If it aborts, a syscall trace identifying which socket operation returns ENOSYS and a comparison with `--security-opt seccomp=unconfined`. These are diagnostic experiments, not confirmed workarounds.
4. If the standalone install succeeds, the cibuildwheel version and relevant configuration, including how it installs uv and invokes the build frontend, are needed to reconstruct the additional conditions.

The successful controls do not establish a uv regression or validate the proposed seccomp/kernel cause.

## Draft response

Linux i686 is a supported Tier 2 target. A minimal wheel install and your full build-requirement install both succeeded here with uv 0.13.6 and Python 3.10.22 on Ubuntu 24.04 x86_64. This environment cannot access Docker or test the affected Ubuntu 26.04/i686 combination, so those results do not rule out the reported failure. Please capture `/usr/local/bin/uv --version` and `file /usr/local/bin/uv` inside the failing container; the host setup-uv version does not identify that binary. Then compare a minimal wheel install on ubuntu-24.04 and ubuntu-26.04 with the container digest and uv version pinned. Please include verbose output and a syscall trace covering socket creation if it aborts. Comparing the default seccomp profile with `--security-opt seccomp=unconfined` would help test the filtering hypothesis; no such comparison has been observed here.

## Classification

The reported uv pip install process aborts with SIGABRT after UnixStream creation returns ENOSYS on Linux i686, a documented Tier 2 target. This is an operational failure, even though the responsible component and syscall remain unconfirmed. No existing issue or pull request was found that tracks the same failure. The container uv version is unknown, so the report does not establish a regression between uv 0.13.6 and 0.13.7 or a recurrence of a previously fixed bug.

Errno 38 is ENOSYS. A seccomp policy difference, kernel compatibility difference, and the precise socket syscall are hypotheses from the report, not confirmed findings. Neither the proposed standalone Docker reproduction nor a rerun on ubuntu-24.04 nor the seccomp comparison has been performed by the reporter. No fix or validated workaround for this exact failure was found.

## Related

No closely related existing issue or pull request was found. The candidates below were inspected and ruled out rather than treated as duplicate or adjacent tracking discussions.

## Supporting evidence

The current issue has the bug label and no comments at the time of inspection.

The platform policy at `docs/reference/policies/platforms.md` lists Linux i686 as Tier 2, publishes both GNU and musl artifacts, and explains that Tier 2 targets are continuously built without running the full test suite. This supports treating the failure as a bug report while avoiding a guarantee that this runner/container combination was tested.

The CLI initializes a Tokio runtime with `enable_all()` in `crates/uv/src/lib.rs` near line 3117. `Cargo.toml` sets the release profile to `panic = "abort"`. These are relevant implementation facts for investigating initialization failures and SIGABRT; they do not establish that this exact diagnostic comes from that runtime or identify the syscall returning ENOSYS. The reproduction work documented above used the installed uv executable; no checkout files were modified and no builds of uv were run.

The 0.13.6 and 0.13.7 release notes were inspected before choosing the fixture. They do not describe a change addressing this UnixStream/i686 failure. The merged diff of astral-sh/uv#23048 updates tokio-rustls from 0.26.5 to 0.26.6 and consolidates Windows dependency references in Cargo.lock; it does not establish a connection to the reported failure. The checkout has shallow history, so the changelog and this read-only pull request inspection supplement local history. A comparison of the affected and last successful container uv versions remains impossible until those versions are identified.

### Candidates inspected and ruled out

| Reference | Evidence and important difference |
| --- | --- |
| astral-sh/uv#22170, closed | Also reports SIGABRT during `uv pip install` and uses a socketpair, but the trace identifies stderr writes returning EAGAIN after inheriting nonblocking I/O. A maintainer attributed it to Bun and closed it after an upstream fix. It does not involve UnixStream creation returning ENOSYS. |
| astral-sh/uv#21935, open | UnixStream appears in a proposed regression test. The actual report is `uv run` forwarding SIGPIPE from a network failure to Python and exiting 141. This differs from uv itself aborting during socket creation. Maintainer comments leave the signal-handling design unresolved. |
| astral-sh/uv#12830, closed, and astral-sh/uv#13017, merged | Following the SIGPIPE discussion leads to historical signal forwarding for child processes. The merged change forwards additional signals in `uv run`; it does not fix a failed socket-creation syscall or establish a regression here. |
| astral-sh/uv#9050, closed | Shares ENOSYS and the exact OS error wording, but concerns acquiring a Python installation lock on a cluster filesystem. The reporter confirmed that administrators had disabled flock. |
| astral-sh/uv#12433, open, and astral-sh/uv#12436, closed | The i686 report and linked debugging PR concern a 32-bit compiler exhausting its address space while building uv. The new issue runs an installed executable and reports a socket-creation error. |
| astral-sh/uv#2732, closed | Also involves i686 and a runtime crash, but under iSH emulation, with division-by-zero, illegal-instruction, and segmentation-fault symptoms. A contributor investigated SSE2 emulation and a maintainer closed it as external and non-actionable. It does not report this ENOSYS failure. |
| astral-sh/uv#21743, closed | SIGABRT during downloads on aarch64 was traced to AWS-LC random-number generation. The architecture, backtrace, and failure operation differ. |
| astral-sh/uv#16859, closed | An unsupported operation on macOS ExFAT was traced to atomic lockfile creation with renameatx_np. It concerns filesystem semantics, not Unix socket creation. |
| astral-sh/uv#16455, open | Socket-related identifiers appear in interpreter configuration output, but the failure is a missing Python math module when invoking an interpreter through nested symlinks. |
| astral-sh/uv#23140, open | Changes uv's official x86_64/aarch64 Docker images to consume release musl archives. It has not merged and does not address a separately installed uv inside a PyPA i686 container. |
| astral-sh/uv#21758, open | Proposes diagnostics when a child process dies from a signal. Here the external Python build frontend reports that uv itself aborted. |
| astral-sh/uv#23048, merged 2026-11-03 | Updates tokio-rustls from 0.26.5 to 0.26.6. Its description supplies no evidence connecting the dependency update to UnixStream initialization or this crash; matching timing alone does not establish causality. |

## Search coverage

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests using authenticated gh. Literal searches covered "failed to create UnixStream", "Function not implemented", UnixStream, ENOSYS, SIGABRT, socketpair, i686/manylinux, ubuntu-26.04, and cibuildwheel. Conceptual searches covered 32-bit failures, container crashes, sockets, Tokio runtime initialization, and signals; separate cause-oriented searches covered seccomp and Docker permissions. Historical/fix searches included uv 0.13.6/0.13.7 crashes and merged runtime/platform changes. Included comments in targeted searches and inspected candidate discussions and linked historical PRs. PR text searches returned no matches; supplemented them by screening 1,500 recent PR titles across all states. Ruled out astral-sh/uv#22170 (nonblocking stderr/EAGAIN), astral-sh/uv#21935 (SIGPIPE forwarding), astral-sh/uv#9050 (disabled flock), astral-sh/uv#12433 (compiler memory exhaustion), and astral-sh/uv#2732 (iSH emulation crashes). The merged signal-forwarding change astral-sh/uv#13017 and open Docker-image change astral-sh/uv#23140 do not track this failure. No matching prior fix or existing regression tracker was found.

Exact identifiers and observable symptoms were searched separately from the reporter's proposed causes. Searches included both issue states, pull request states including merged changes, and targeted title/body/comment searches. A temporary search rate limit was allowed to reset and the affected searches were retried. Pull request keyword searches returned no matches even for broad terms, so coverage was supplemented with the 1,500 most recent pull request titles across all states and direct inspection of relevant historical references. This is a search limitation, not proof that no related historical pull request exists.

## Reporter-provided run references

Failing job: https://github.com/o-murphy/rp2040py/actions/runs/42415595170/job/128532856289

Last successful run: https://github.com/o-murphy/rp2040py/actions/runs/40858286448

These links are reporter-provided leads. Their raw logs were not downloaded or independently validated during this triage.
