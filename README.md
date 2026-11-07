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

## Draft response

Linux i686 is a supported Tier 2 target. Your log shows uv aborting while creating a UnixStream, but it does not identify the failing syscall or establish a Docker/seccomp cause. Please capture `/usr/local/bin/uv --version` and `file /usr/local/bin/uv` inside the failing container; the host setup-uv version does not identify that binary. Then compare a minimal wheel install on ubuntu-24.04 and ubuntu-26.04 with the container digest and uv version pinned. If it still aborts, please share verbose output and a syscall trace covering socket creation. Comparing the default seccomp profile with `--security-opt seccomp=unconfined` would also help test the filtering hypothesis.

## Classification

The reported uv pip install process aborts with SIGABRT after UnixStream creation returns ENOSYS on Linux i686, a documented Tier 2 target. This is an operational failure, even though the responsible component and syscall remain unconfirmed. No existing issue or pull request was found that tracks the same failure. The container uv version is unknown, so the report does not establish a regression between uv 0.13.6 and 0.13.7 or a recurrence of a previously fixed bug.

Errno 38 is ENOSYS. A seccomp policy difference, kernel compatibility difference, and the precise socket syscall are hypotheses from the report, not confirmed findings. Neither the proposed standalone Docker reproduction nor a rerun on ubuntu-24.04 nor the seccomp comparison has been performed by the reporter. No fix or validated workaround for this exact failure was found.

## Related

No closely related existing issue or pull request was found. The candidates below were inspected and ruled out rather than treated as duplicate or adjacent tracking discussions.

## Supporting evidence

The current issue has the bug label and no comments at the time of inspection.

The platform policy at `docs/reference/policies/platforms.md` lists Linux i686 as Tier 2, publishes both GNU and musl artifacts, and explains that Tier 2 targets are continuously built without running the full test suite. This supports treating the failure as a bug report while avoiding a guarantee that this runner/container combination was tested.

The CLI initializes a Tokio runtime with `enable_all()` in `crates/uv/src/lib.rs` near line 3117. `Cargo.toml` sets the release profile to `panic = "abort"`. These are relevant implementation facts for investigating initialization failures and SIGABRT; they do not establish that this exact diagnostic comes from that runtime or identify the syscall returning ENOSYS. No code in the checkout was changed and no reproduction or build was run.

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

## Next diagnostic steps

1. Identify the executable inside the failing container with `/usr/local/bin/uv --version` and `file /usr/local/bin/uv`.
2. Pin that uv version and the exact container digest, then run a minimal `uv pip install --python /opt/python/cp310-cp310/bin/python wheel` on both runner versions. This separates the runner change from an unrecorded uv update and the full build dependency set.
3. Capture verbose output and a syscall trace around the failure, including socket/socketpair/socketcall operations where applicable. A panic backtrace, if available, would help identify the initialization site.
4. Compare default seccomp with `--security-opt seccomp=unconfined` as a diagnostic experiment. A successful comparison would support further investigation of filtering; no such result is currently available.

## Reporter-provided run references

Failing job: https://github.com/o-murphy/rp2040py/actions/runs/42415595170/job/128532856289

Last successful run: https://github.com/o-murphy/rp2040py/actions/runs/40858286448

These links are reporter-provided leads. Their raw logs were not downloaded or independently validated during this triage.
