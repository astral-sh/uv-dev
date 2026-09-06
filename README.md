# [Torch] Support official ROCm repository on Windows

Issue: astral-sh/uv#21471

Classification: duplicate

## Summary

The report asks uv to support AMD's newly official ROCm Torch wheel repository on Windows. The
current upstream PyTorch ROCm repository is Linux-only, while AMD's Windows packaging requires the
`device-all` extra for both `torch` and `torchvision`.

The closest existing tracker is astral-sh/uv#16017. It already requests support for AMD-hosted ROCm
wheel indexes because the indexes on download.pytorch.org do not cover all required AMD GPU
distributions. astral-sh/uv#21471 updates that request with official stable Windows availability and
a different package-selection convention, but the underlying uv capability is the same: choosing an
AMD-hosted ROCm source instead of the PyTorch-hosted ROCm source.

Earlier work added explicit ROCm backend values and Linux AMD GPU auto-detection, but did not add
AMD-hosted repositories or Windows ROCm selection. No evidence indicates a previously supported
Windows path has regressed.

A maintainer has since indicated that the preferred outcome is for AMD's Windows wheels to be
published on the official PyTorch index instead of adding a separate AMD repository path to uv.
They also questioned whether uv should special-case the reported `[device-all]` requirement. The
reason AMD uses this extra and whether it is technically necessary remain unresolved in the issue.

## Draft response

Thanks. This is the Windows/stable form of the AMD-hosted ROCm-index request already tracked in
astral-sh/uv#16017. uv currently routes ROCm backends to download.pytorch.org and only selects ROCm
index candidates automatically on Linux, so supporting AMD's Windows repository, including the
`[device-all]` convention, requires extending that backend logic rather than fixing a regression.

Let's keep the implementation discussion in astral-sh/uv#16017. Please add the exact AMD index URL
and a minimal pip command that succeeds on Windows there; those details will help define the
expected behavior.

## Classification

This is a duplicate of the open astral-sh/uv#16017 because both issues request support for
AMD-hosted ROCm wheel indexes where the PyTorch-hosted indexes are insufficient. The older report
uses AMD nightly flat indexes on Ubuntu and focuses on GPU-architecture coverage; the new report
uses the official stable repository on Windows and identifies `[device-all]` as a packaging
requirement. Those are meaningful updated implementation constraints, but they can be centralized
under the same open repository-selection request.

The source confirms this is an unimplemented extension rather than incorrect established behavior:

- `TorchBackend` maps every explicit ROCm backend to a download.pytorch.org URL.
- Automatic AMD selection enumerates ROCm backends only for manylinux and musllinux; an AMD
  accelerator on Windows currently receives the CPU index.
- AMD auto-detection uses `rocm_agent_enumerator`; Windows device-tree detection currently covers
  Intel GPUs, not AMD GPUs.
- The PyTorch guide describes ROCm builds and source markers as Linux-only.

Duplicate takes precedence over `enhancement`. There is no evidence of a regression: the official
Windows distribution described in astral-sh/uv#21471 is newer than the existing Linux-only support.

## Maintainer direction

A uv maintainer prefers getting the Windows ROCm wheels upstreamed to the official PyTorch index
rather than teaching uv to select AMD's separate repository. This makes upstream publication the
current preferred direction; no maintainer approval has been given for the proposed uv change.

The maintainer also expressed reluctance to have uv rewrite `torch` and `torchvision` requirements
to include `[device-all]` without understanding AMD's packaging choice. Investigation should first
establish why the extra is required, what dependencies it selects, and whether AMD or PyTorch can
publish metadata that avoids installer-specific rewriting.

## Related

- astral-sh/uv#16017 — Open. This is the canonical prior request to use AMD-hosted ROCm wheel
  indexes when download.pytorch.org lacks the required distributions. Its Ubuntu/nightly and
  GPU-architecture focus differs from the new official Windows packaging details, but the
  repository-selection capability is the same.
- astral-sh/uv#14086 — Closed as completed. This tracked automatic ROCm detection for
  `--torch-backend`; its discussion and fix are based on Linux GPU architectures and establish the
  subsystem that Windows support would extend.
- astral-sh/uv#14120 — Merged. This added explicit ROCm values to `--torch-backend` and hard-coded
  their download.pytorch.org index URLs. Supporting AMD's Windows repository would augment this
  index model.
- astral-sh/uv#14176 — Merged. This added AMD GPU auto-detection through
  `rocm_agent_enumerator`, with a Linux ROCm installation test. It did not implement Windows AMD
  detection or Windows-specific index selection.

## Search coverage

Literal searches covered `ROCm Windows`, `official ROCm`, `device-all`, `TheRock`, AMD repository,
and Windows PyTorch-index terms. No earlier item used the exact new identifiers; exact
`device-all` results only returned astral-sh/uv#21471.

Conceptual searches covered ROCm, PyTorch and Torch indexes, `torch-backend`, AMD GPU detection,
automatic backend selection, repository fallback, and GPU wheel variants across open and closed
issues and open, closed, and merged pull requests. Fix-oriented inspection followed the historical
ROCm backend and auto-detection issues to astral-sh/uv#14120 and astral-sh/uv#14176 and checked later
ROCm-version additions.

astral-sh/uv#16522 was inspected but ruled out because it asks for generalized backend-aware wheel
selection for packages beyond PyTorch. astral-sh/uv#18844 and astral-sh/uv#10712 were also ruled out:
they concern manual Linux index configuration and transitive Triton packages, not selecting AMD's
Windows repository or applying the `device-all` extra.
