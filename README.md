# statically linked uv segfaults on riscv64

Issue: astral-sh/uv#21151

Classification: bug

## Summary

The reported crash is reproducible with the official, statically linked uv 0.12.5
`riscv64gc-unknown-linux-musl` release artifact. Under QEMU 8.2.2, `uv tool run nox --version`
(the canonical form of `uvx nox --version`) terminates with SIGSEGV and exit status 139 while
making an unauthenticated HTTPS request to PyPI. The same artifact also reaches the reported point
of failure in an Alpine 3.24.0 riscv64 root filesystem with musl 1.2.6 and Python 3.14.7.

The reporter additionally reproduced uv 0.12.4 on QEMU and real hardware, reproduced uv 0.12.5
under QEMU, and showed that the riscv64 glibc artifact completes in an otherwise controlled
comparison. The independent reproduction below confirms the musl/static failure but did not repeat
the riscv64 glibc comparison. An x86_64 GNU uv 0.12.5 control completed the same tool invocation.

No earlier issue or pull request tracks this runtime failure. The nearest history is the request to
publish riscv64 musllinux binaries in astral-sh/uv#16063 and its implementation in
astral-sh/uv#18228. The new issue should remain the canonical discussion for the crash.

## Reproduction

Outcome: **reproducible**.

The reproduction used only temporary directories and the checksummed official uv 0.12.5 release
archive. The runner itself was x86_64 Ubuntu, so the riscv64 binary was executed with Ubuntu's
QEMU user emulator 8.2.2. A minimal signal-confirming invocation was:

```console
$ export UV_CACHE_DIR="$RUNNER_TEMP/uv-21151/cache"
$ export UV_TOOL_DIR="$RUNNER_TEMP/uv-21151/tools"
$ export UV_PYTHON_INSTALL_DIR="$RUNNER_TEMP/uv-21151/python"
$ qemu-riscv64 uv-riscv64gc-unknown-linux-musl/uv -vv tool run nox --version
...
TRACE Attempting unauthenticated request for https://files.pythonhosted.org/.../nox-2026.8.10-py3-none-any.whl.metadata
Segmentation fault (core dumped)
$ echo $?
139
```

`file` identifies this release binary as a statically linked, stripped 64-bit RISC-V ELF. A second,
smaller fixture that avoids executing the tool's Python entry point also exited 139 on the initial
PyPI request:

```console
$ qemu-riscv64 uv-riscv64gc-unknown-linux-musl/uv -vv pip install \
    --target "$RUNNER_TEMP/uv-21151/target" \
    --python-version 3.14 \
    --python-platform riscv64-unknown-linux \
    --only-binary :all: nox
...
TRACE Attempting unauthenticated request for https://pypi.org/simple/nox/
Segmentation fault (core dumped)
$ echo $?
139
```

The release artifact was also run through PRoot in an Alpine 3.24.0 riscv64 minirootfs. That fixture
reported `riscv64`, Alpine 3.24.0, musl 1.2.6, Python 3.14.7, and
`uv 0.12.5 (riscv64gc-unknown-linux-musl)`. It progressed to an unauthenticated
`files.pythonhosted.org` request and then stopped at the same point. PRoot did not preserve the
guest signal status, so the direct QEMU invocations above provide the exit-139 confirmation.

As a control, the installed `uv 0.12.5 (x86_64-unknown-linux-gnu)` completed
`uv tool run nox --version` against isolated caches and printed `2026.8.10` with exit status 0.

The integration tests in `crates/uv/tests/tool/tool_run.rs` exercise tool resolution and execution,
and `crates/uv-client/tests/it/ssl_certs.rs` exercises HTTPS behavior, but neither provides a
riscv64 static-musl release-binary test. The release workflow's Alpine riscv64 smoke test in
`.github/workflows/build-release-binaries.yml` runs only `uv --help` and `uvx --help`, so it does
not cover the failing HTTPS download path.

## Report and supporting evidence

- Independently observed behavior: uv reaches an unauthenticated request to PyPI and exits with
  `Segmentation fault (core dumped)` and status 139.
- Trigger: the riscv64 musl/static release artifact. The report reproduces with the PyPI wheel and
  the standalone installer on Alpine 3.24.
- Controls: the reporter's riscv64 glibc wheel succeeds in the same manylinux/QEMU environment;
  independently, the installed x86_64 GNU build succeeds with the same tool request.
- Versions: uv 0.12.4 and 0.12.5 are affected, so there is no indication that a later released fix
  has already resolved it.
- Release configuration: astral-sh/uv#18228 added the exact
  `riscv64gc-unknown-linux-musl` target. The current workflow explicitly enables static CRT linkage
  for it and gives the wheel both `musllinux_1_1_riscv64` and `manylinux_2_31_riscv64`
  compatibility. Its Alpine test runs `uv --help` and `uvx --help`, which do not exercise the
  failing network path.
- Confirmed scope does not establish a root cause. Static linkage, musl, and the release build are
  distinguishing conditions; the crashing component remains unknown until a backtrace or further
  isolation identifies it.

The reporter's proposed removal of the manylinux compatibility tag could prevent this static
artifact from being selected on glibc systems. It would not correct the demonstrated crash on
Alpine, so it is a possible packaging mitigation rather than an established root fix.

## Draft response

Thanks for the detailed reproductions. We independently reproduced the SIGSEGV with the official
uv 0.12.5 `riscv64gc-unknown-linux-musl` artifact under QEMU: the process exits 139 during an
unauthenticated PyPI HTTPS request. We also reached the same point in an Alpine 3.24.0 riscv64
rootfs with musl 1.2.6 and Python 3.14.7. The installed x86_64 GNU uv 0.12.5 build completes the
same tool request. Together with your real-hardware result and working riscv64 glibc control, this
confirms a bug specific to the riscv64 musl/static release artifact rather than a QEMU-only issue.

The affected artifact was added in astral-sh/uv#18228, and its current Alpine smoke test only
exercises help output, so it does not cover the HTTPS path that crashes here.

The next useful diagnostic is a native backtrace from the core dump, ideally with debug symbols;
please attach one if available. Using the glibc wheel is the current workaround. Removing the
manylinux compatibility tag may avoid selecting this artifact on glibc systems, but it would not
fix the Alpine crash, so we should not treat that tag change as the root fix without identifying
the fault.

## Classification

This is a bug because official uv release artifacts reproducibly terminate with SIGSEGV during
ordinary package retrieval. The working glibc build establishes that the command and package are
not inherently failing, while the real-hardware reproduction distinguishes this report from a
QEMU-only limitation. The independent exit-139 reproduction confirms the symptom but not the root
cause. The source confirms how the musl artifact is built and tested but does not yet confirm why
it crashes. There is no open issue or pull request already tracking the same failure, so this is
not a duplicate. It is also not primarily an enhancement: changing the wheel tag is a proposed
mitigation for incorrect runtime behavior.

## Related

- astral-sh/uv#16063 — **RISC-V64 musllinux binaries** (closed). This is the canonical request for
  the exact riscv64 musllinux artifact. It was closed when the artifact became available and does
  not report or diagnose the segmentation fault.
- astral-sh/uv#18228 — **Add riscv64 musl target to build-release-binaries workflow** (merged).
  This change introduced the exact static musl release target and combined compatibility tagging
  implicated by the report. Its discussion concerns publishing and tag acceptance, not this
  runtime crash, and it is historical provenance rather than an existing bug tracker.

## Search coverage

Authenticated repository-wide searches covered open and closed issues and open, closed, and merged
pull requests. Literal searches included `segmentation fault`, `segfault`, `core dumped`,
`riscv64`, `riscv64gc-unknown-linux-musl`, `musllinux`, `manylinux_2_31_riscv64`, `Alpine`,
`uvx`, `unauthenticated request`, and `statically linked`. Conceptual and fix-oriented searches
covered static/musl release artifacts, network and download crashes, TLS-related terms, RISC-V
release support, wheel compatibility tags, and historical platform fixes. The chain from
astral-sh/uv#10883 to astral-sh/uv#16063 and astral-sh/uv#18228 was inspected, along with the
earlier glibc release support and riscv64 wheel-installation changes.

astral-sh/uv#16024 was the most plausible same-symptom candidate but was ruled out: it affects
amd64 glibc under QEMU, also crashes `rustc`, and has no native-hardware reproduction. In contrast,
astral-sh/uv#21151 reproduces on real riscv64 hardware, gets as far as network activity, and
distinguishes the failing musl/static build from a working glibc build. astral-sh/uv#11231 was also
ruled out because its segmentation fault occurs in an emulated Ubuntu Python package installation
inside release-test infrastructure, not in uv's riscv64 musl network path. No merged fix or closed
same-problem report was found for uv 0.12.4 or 0.12.5.
