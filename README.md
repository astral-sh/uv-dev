# nvidia detection beyond `nvidia-smi`

Issue: astral-sh/uv#22855

Classification: duplicate

## Summary

Duplicate of astral-sh/uv#14664, with an open ioctl prototype in astral-sh/uv#14743. Merged astral-sh/uv#22690 provides the current wheel-variant implementation; astral-sh/uv#14742 documents the compute-capability selection problem. The historical multi-GPU parsing failure was fixed separately.

The reporter wants fast, reliable NVIDIA capability discovery for automatic Torch backend selection and wheel variants, including environments where nvidia-smi is absent. The report is a design/context tracker, with proposed Linux, Windows, and WSL probes; it supplies no failing uv command or affected uv release. Its detailed binary-inspection claims and proposed algorithm have not been independently validated.

## Report and scope

- Detect the supported CUDA version without requiring nvidia-smi or loading a driver library into a statically linked uv executable.
- Account for cuda-compat and the user-mode library actually used by the target Python environment, rather than treating the kernel driver version as that library's CUDA API version.
- Obtain GPU compute capability for wheel `sm_arch` matching, including GPU visibility and heterogeneous-device concerns.
- Consider native Windows, WSL, containers, Jetson, cross-installation, and explicit capability overrides. The report proposes ELF notes, DLL version resources, hardware mappings, and environment-variable handling; these are proposals, not established portable APIs or accepted implementation requirements.

## Draft response

This detection work is already tracked in astral-sh/uv#14664, including static builds, cuda-compat environments without NVML or nvidia-smi, and compute-capability discovery. The provider added in astral-sh/uv#22690 still uses nvidia-smi for CUDA-version and compute-capability detection; astral-sh/uv#14743 explores direct kernel-driver queries. Please add your platform-specific findings to astral-sh/uv#14664 so we can keep the API and support-scope discussion together.

## Classification

Open astral-sh/uv#14664 already tracks the same unresolved request, including the static-linking, missing userspace tools, cuda-compat, and compute-capability constraints. The new platform-specific proposals extend that discussion and can be centralized there. No return of the separately fixed multiline-output bug is established; astral-sh/uv#22690 predates this report and does not solve the requested detection improvements.

The canonical issue's title suggests NVML, but its discussion is broader: it explicitly considers why NVML alone cannot cover these environments and explores alternative driver queries, subprocesses, and capability mappings. This is substantive overlap in the requested behavior and constraints, not merely shared NVIDIA terminology. The duplicate classification takes precedence over classifying the proposed improvements as an enhancement.

## Related

- astral-sh/uv#14664 — Use `nvml` to detect CUDA driver version (issue, open). Canonical discussion of the same detection request. Comments cover static musl builds that cannot dlopen, working cuda-compat containers without NVML or nvidia-smi, Python subprocess costs, WSL, and compute-capability discovery through driver interfaces or hardware mappings.

- astral-sh/uv#14743 — Directly ask the NVIDIA kernel driver what version it is (pull request, open). Prototype linked from astral-sh/uv#14664 adds a /dev/nvidiactl ioctl probe before nvidia-smi, targeting speed and installations without the CLI. Its patch detects the kernel driver version only; compute-capability queries remain discussion, and it does not resolve cuda-compat user-mode version selection.

- astral-sh/uv#22690 — Implement a vendored nvidia variant provider (pull request, merged). Merged on October 22, 2026, before this report, and introduced the relevant wheel-variant provider. Source still queries nvidia-smi for CUDA user-mode version and compute capability. Review explicitly discusses missing nvidia-smi, static-build dlopen constraints, and deferred CUDA_VISIBLE_DEVICES filtering; this is implementation context, not a completed fix.

- astral-sh/uv#14742 — `--torch-backend=auto` ignores CUDA compute capability when installing `torch` for GTX 1080 Ti (issue, open). Demonstrates why compute capability matters: automatic selection installs cu128 wheels that cannot run on an sm_61 GPU, while cu126 works. Maintainer discussion considers NVML. It concerns compatibility-aware Torch wheel selection, whereas this report focuses on obtaining capabilities reliably.

- astral-sh/uv#14647 — `--torch-backend=auto` fails on systems with multiple GPUs and without `/proc/driver/nvidia/version` (issue, closed). Historical WSL/multi-GPU failure in the same nvidia-smi fallback: repeated driver-version lines caused a version parse error. Fixed by astral-sh/uv#15460. This report requests broader detection and does not show that parsing bug recurring.

- astral-sh/uv#15460 — Respect multi-GPU outputs in nvidia-smi (pull request, merged). Merged November 2, 2025, fixing astral-sh/uv#14647; current source parses the first driver-version line. The author explicitly omitted NVIDIA_VISIBLE_DEVICES masking. It fixes multiline parsing, not CLI-independent detection or GPU-visibility handling.

## Supporting evidence

The source was inspected at checkout commit `f8bc5d19484d4ea8edb54c49dac7e2810e5c343b`.

- `crates/uv-torch/src/nvidia.rs:62`: kernel-driver detection reads `/sys/module/nvidia/version`, then `/proc/driver/nvidia/version`, then queries nvidia-smi. These file probes do not provide GPU compute capability or the effective CUDA user-mode version.
- `crates/uv-torch/src/accelerator.rs:100`: Torch accelerator selection calls the kernel-driver detector directly. This differs from the wheel-variant provider's fuller capability probe.
- `crates/uv-torch/src/nvidia.rs:22`: the fuller probe requires successful `nvidia-smi -q` output containing a CUDA version even after file-based kernel-driver detection succeeds. It queries `compute_cap` separately and takes the maximum parsed capability. The `CUDA_VISIBLE_DEVICES` filter remains an explicit TODO.
- `crates/uv-distribution/src/variant/nvidia.rs:88`: the vendored provider recognizes both `NV_VARIANT_PROVIDER_FORCE_CUDA_DRIVER_VERSION` and `NV_VARIANT_PROVIDER_FORCE_SM_ARCH`. When both valid overrides are supplied, the lazy hardware probe is unnecessary. Partial overrides can still invoke the full probe for the remaining property; the report's proposed fully independent probes are not current behavior.
- `crates/uv-torch/src/nvidia.rs:112`: driver-version parsing reads only the first output line. A nearby `nvidia_smi_multi_gpu` test covers repeated version lines, supporting the distinction from the historical fixed failure.
- In astral-sh/uv#14664, geofft's July 21, 2025 comments describe and demonstrate working cuda-compat containers with no NVML or nvidia-smi. zanieb's July 24 comment confirms the fully static ELF limitation. These directly support the report's constraints.
- In astral-sh/uv#14743, the patch adds a kernel-version ioctl probe on `/dev/nvidiactl`. Subsequent comments explore architecture mappings and a possible SM-version query, without an implemented compute-capability solution.
- In astral-sh/uv#22690, October 22, 2026 review comments explicitly discuss absent nvidia-smi and static-build constraints, and defer GPU-visibility handling. It merged at 22:14 UTC; astral-sh/uv#22855 opened at 23:00 UTC. The provider is therefore preceding implementation context, not a fix created in response to this issue.

## Search coverage and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separate literal searches covered nvidia-smi, compute capability, cuda-compat, /sys/module/nvidia/version, libcuda.so.1, nvcuda.dll, sm_arch, CUDA_VISIBLE_DEVICES, and both NV_VARIANT_PROVIDER_FORCE overrides. Conceptual searches covered NVML, CUDA/dlopen/static builds, GPU detection, forward compatibility, WSL, torch-backend auto, and wheel variants. Checked closed detection reports and historical fixes; followed comments, review threads, and timeline references to astral-sh/uv#14743, astral-sh/uv#15460, and astral-sh/uv#22690. PR keyword searches returned no hits even for known matching PRs, so supplemented them with a 1,000-PR all-state listing and direct reads of linked PRs. Ruled out astral-sh/uv#17236 as a duplicate: uv pip detection worked, and the remaining problem was unsupported torch-backend configuration in uv sync. Broader wheel-variant discussions astral-sh/uv#16522 and astral-sh/uv#22043 were less specific.

The original backend implementation, astral-sh/uv#12070, points to astral-sh/uv#14664 as the follow-up discussion. The wheel-variant tracker, astral-sh/uv#22043, points to astral-sh/uv#22690. These reference chains support the selected canonical issue and current implementation context.

The historical parsing fix and current first-line parser provide no evidence of a regression. No GPU reproduction or platform-specific binary probe was run for this handoff. The proposed reply is for maintainer review; no GitHub changes were made.
