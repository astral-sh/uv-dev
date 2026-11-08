# uv aborts (SIGABRT, "failed to create UnixStream: Function not implemented") in the i686 manylinux container on GitHub's ubuntu-26.04 runner

Issue: astral-sh/uv#23147

Classification: bug

## Summary

A uv maintainer attributes this failure to the upstream Docker/seccomp bug tracked in actions/runner-images#14790 and identifies Docker 29.4.3 as containing the fix. The upstream discussion reports a reproduced socketcall denial returning ENOSYS in i386 glibc containers on ubuntu-26.04 with Docker 29.4.2. This closely matches the reported architecture, host, Docker version, and error. The runner-image update in actions/runner-images#15205 remains open; availability of a fixed Docker release does not establish that the affected hosted runner image has received it. The exact uv workload has not been retested with a fixed daemon in the available evidence.

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

1. The Docker server version on the affected runner and the result of retrying the original workload with Docker 29.4.3 or later. The maintainer's upstream attribution makes this the first comparison to perform; no such uv retest is currently available.
2. The version and architecture of `/usr/local/bin/uv` inside the failing container, and preferably the corresponding values from the successful run. Host setup-uv versions do not provide this information.
3. If the failure persists with a fixed daemon, run the minimal wheel install on the affected Ubuntu 26.04 runner using the pinned container digest and pinned container uv, with verbose output. Compare the same inputs on Ubuntu 24.04 to isolate the runner change.
4. If it aborts, capture a syscall trace identifying which socket operation returns ENOSYS and compare with `--security-opt seccomp=unconfined`. This comparison is validated in the upstream Debian reproduction, but has not been tested for this uv workload.
5. If the standalone install succeeds, the cibuildwheel version and relevant configuration, including how it installs uv and invokes the build frontend, are needed to reconstruct the additional conditions.

The successful local controls do not establish a uv regression or independently validate the upstream diagnosis for the affected uv workload.

## Investigation priority

First verify the Docker **server** version on the affected runner and retry the original i686 workload with Docker 29.4.3 or later, which the uv maintainer and upstream discussion identify as carrying the seccomp fix. Keep the container digest and container uv version recorded for the comparison. No successful retest of this exact uv workload on a fixed daemon has been supplied.

The upstream runner-image issue is still open, and actions/runner-images#15205 proposes updating Ubuntu 26.04 to Docker 29.7.2 but has not merged. Do not assume that changing or rerunning the runner label selects a fixed image; inspect the actual daemon version. If the failure persists with a fixed daemon, return to the pinned minimal reproduction and socket syscall diagnostics below.

## Classification

Classification remains bug. The reported uv pip install process aborts with SIGABRT after UnixStream creation returns ENOSYS on Linux i686, a documented Tier 2 target. The uv maintainer now attributes the failure to an upstream Docker bug with a fix in Docker 29.4.3. This changes the investigation priority to daemon/runner compatibility; it does not establish a uv implementation regression or a need for a uv code change. The crashing container's uv version remains unknown.

The upstream socketcall/seccomp mechanism is supported by the linked discussion's reported reproduction and source analysis. There is still no syscall trace or fixed-daemon retest of this exact uv workload, so the connection is a maintainer attribution supported by a strong upstream match rather than a locally verified diagnosis. The initial x86_64 controls and their needs_more_information outcome remain accurate for the reproduction work performed here.

## Related

- actions/runner-images#14790 — Open: “Ubuntu 26.04: Docker 29.4.2 blocks socketcall, so i386 containers have no network.” The uv maintainer links this as the upstream cause. An upstream contributor reports reproducing the default-seccomp failure on a real ubuntu-26.04 runner, success with seccomp unconfined, and success after upgrading the daemon to a build carrying seccomp v0.2.3. The reproduction uses i386 Debian/glibc name resolution, whereas this issue aborts during UnixStream creation in manylinux i686; the matching socketcall denial, ENOSYS, Docker version, and runner make this directly relevant.
- actions/runner-images#15205 — Open, not merged: “[Ubuntu 26.04] Update Docker to 29.7.2.” Proposes the runner-image Docker upgrade and explicitly fixes actions/runner-images#14790. Its author reports i386 Debian networking working after upgrading Docker to 29.7.2. This tracks delivery of a fixed daemon to the runner image, not a uv change, and does not establish completed image rollout.

## Supporting evidence

The issue discussion includes a 2026-11-08 comment from uv maintainer charliermarsh identifying the failure as apparently upstream and resolved in Docker 29.4.3. The comment links to the upstream confirmation in actions/runner-images#14790 (comment ID 5868616762). This supersedes the initial finding that no relevant upstream fix was known.

### Upstream mechanism and fix evidence

The linked upstream comment from v-davit-ioramashvili reports source verification and a live reproduction on ubuntu-26.04:

- Docker 29.4.2 vendors moby/profiles/seccomp v0.2.2, which denies socketcall with ENOSYS. Docker 29.4.3 moves to seccomp v0.2.3, reverting that denial.
- The comment explains that Debian i386 glibc socket wrappers use socketcall because of their minimum-kernel baseline. Its i386 Alpine/musl control is unaffected. This is evidence for the upstream reproduction; the exact libc wrapper path in the reported manylinux uv executable has not been independently established here.
- `docker run --rm i386/debian:bookworm getent hosts github.com` exits 2 with Docker 29.4.2 and default seccomp, and exits 0 with `--security-opt seccomp=unconfined`.
- After upgrading and restarting the daemon with seccomp v0.2.3, the default-seccomp Debian case exits 0. These are upstream-reported results, distinct from the local x86_64 uv controls above.

Docker 29.4.3 or later is therefore a concrete fix candidate for this uv failure, supported by the maintainer attribution and upstream results. Disabling seccomp is an upstream-validated diagnostic comparison for the Debian case; it has not been validated for the exact uv workload. The upstream issue and runner-image PR were both open when inspected. No fixed-image rollout date or successful reporter retest is established.

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

The initial repository-scoped investigation searched astral-sh/uv open and closed issues and open, closed, and merged pull requests using authenticated gh. Literal searches covered "failed to create UnixStream", "Function not implemented", UnixStream, ENOSYS, SIGABRT, socketpair, i686/manylinux, ubuntu-26.04, and cibuildwheel. Conceptual searches covered 32-bit failures, container crashes, sockets, Tokio runtime initialization, and signals; separate cause-oriented searches covered seccomp and Docker permissions. Historical/fix searches included uv 0.13.6/0.13.7 crashes and merged runtime/platform changes. Included comments in targeted searches and inspected candidate discussions and linked historical PRs. PR text searches returned no matches; supplemented them by screening 1,500 recent PR titles across all states. Ruled out astral-sh/uv#22170 (nonblocking stderr/EAGAIN), astral-sh/uv#21935 (SIGPIPE forwarding), astral-sh/uv#9050 (disabled flock), astral-sh/uv#12433 (compiler memory exhaustion), and astral-sh/uv#2732 (iSH emulation crashes). The merged signal-forwarding change astral-sh/uv#13017 and open Docker-image change astral-sh/uv#23140 do not track this failure. No matching prior uv fix or uv regression tracker was found in that initial repository-scoped search. The subsequent maintainer-linked upstream issue and runner-image PR are documented in Related.

Exact identifiers and observable symptoms were searched separately from the reporter's proposed causes. Searches included both issue states, pull request states including merged changes, and targeted title/body/comment searches. A temporary search rate limit was allowed to reset and the affected searches were retried. Pull request keyword searches returned no matches even for broad terms, so coverage was supplemented with the 1,500 most recent pull request titles across all states and direct inspection of relevant historical references. This is a search limitation, not proof that no related historical pull request exists.

## Reporter-provided run references

Failing job: https://github.com/o-murphy/rp2040py/actions/runs/42415595170/job/128532856289

Last successful run: https://github.com/o-murphy/rp2040py/actions/runs/40858286448

These links are reporter-provided leads. Their raw logs were not downloaded or independently validated during this triage.
