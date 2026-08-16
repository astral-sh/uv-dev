# statically linked uv segfaults on riscv64

Issue: astral-sh/uv#21151

Classification: bug

## Summary

The reporter shows that `uvx -vv nox --version` terminates with a segmentation fault while
downloading a wheel on Alpine 3.24 riscv64. The failure occurs with uv 0.12.4 on both QEMU and real
hardware and with uv 0.12.5 under QEMU. A controlled comparison on the same riscv64 environment
shows that the statically linked `riscv64gc-unknown-linux-musl` artifact crashes while the glibc
`manylinux_2_31_riscv64` artifact completes. The installer-provided musl binary fails in the same
way.

No earlier issue or pull request tracks this runtime failure. The nearest history is the request to
publish riscv64 musllinux binaries in astral-sh/uv#16063 and its implementation in
astral-sh/uv#18228. The new issue should remain the canonical discussion for the crash.

## Report and supporting evidence

- Actual behavior: uv reaches an unauthenticated request to `files.pythonhosted.org` and exits with
  `Segmentation fault (core dumped)`.
- Trigger: the riscv64 musl/static release artifact. The report reproduces with the PyPI wheel and
  the standalone installer on Alpine 3.24.
- Control: the riscv64 glibc wheel succeeds in the same manylinux/QEMU environment.
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

Thanks for the detailed reproductions. The affected riscv64 musl release artifact was added in
astral-sh/uv#18228, and its current Alpine smoke test only exercises help output, so it does not
cover the network path that crashes here. Since you reproduced this on real hardware as well as
QEMU and the riscv64 glibc artifact works, we'll treat this as a bug in the musl/static release
artifact rather than a QEMU-only failure.

The next useful diagnostic is a native backtrace from the core dump, ideally with debug symbols;
please attach one if available. Using the glibc wheel is the current workaround. Removing the
manylinux compatibility tag may avoid selecting this artifact on glibc systems, but it would not
fix the Alpine crash, so we should not treat that tag change as the root fix without identifying
the fault.

## Classification

This is a bug because official uv release artifacts reproducibly terminate with SIGSEGV during
ordinary package retrieval. The working glibc build establishes that the command and package are
not inherently failing, while the real-hardware reproduction distinguishes this report from a
QEMU-only limitation. The source confirms how the musl artifact is built and tested but does not
yet confirm why it crashes. There is no open issue or pull request already tracking the same
failure, so this is not a duplicate. It is also not primarily an enhancement: changing the wheel
tag is a proposed mitigation for incorrect runtime behavior.

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
