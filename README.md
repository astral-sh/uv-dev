# --torch-backend=auto on Windows with CUDA 12.x driver resolves torchvision 0.2.0 (2018)

Issue: astral-sh/uv#22965

Classification: bug

## Summary

The reporter says unpinned `uv pip install torch torchvision --torch-backend=auto` selects `torch 2.9.0+cu129` and `torchvision 0.2.0` on Windows with NVIDIA drivers >=528.33 and <580. Installation and basic imports succeed, but modern torchvision symbols and operations are missing. The report covers uv 0.13.6 and 0.12.16, Python 3.10–3.13, an RTX 3060 Ti, and simulated driver version 560.94. Linux CUDA 12.x and Windows CUDA 13 are reported unaffected.

astral-sh/uv#15416 introduced the relevant cu129 priority. astral-sh/uv#12357 and astral-sh/uv#12385 cover earlier fallback behavior; astral-sh/uv#17649, astral-sh/uv#8157, and astral-sh/uv#9843 cover pathological backtracking to permissive old releases. No exact duplicate or confirmed regression was found.

Source inspection and public wheel metadata support the reported index and dependency interaction. The exact commands and runtime failures were not independently reproduced; no uv executable was available on PATH, and no build was performed.

## Draft response

The Windows backend table does prefer cu129 for this driver range. Its Windows wheels include torch 2.9.0, while torchvision 0.23.0 requires torch 2.8.0; the legacy torchvision 0.2.0 wheel has no torch version bound.

One distinction from astral-sh/uv#12357: astral-sh/uv#12385 already enabled consideration of later compatible indexes. That code is still present, but resolution can accept an old candidate from the preferred index. This needs a focused test of backend ordering and legacy-wheel selection before choosing a fix.

For now, use your explicit --torch-backend=cu128 workaround and set a torchvision lower bound appropriate for the APIs your application uses. The reported torchvision>=0.3 workaround excludes these legacy wheels, but does not guarantee support for every modern API.

## Classification

Automatic backend selection reportedly succeeds with a severely obsolete torchvision despite newer matched Windows pairs on other driver-compatible indexes. The cu129-first ordering and relevant wheel metadata are independently confirmed. The existing open resolver analogue does not cover automatic backend ordering, and the historical fallback and backtracking fixes do not establish that this specific case previously worked. Classify as a distinct bug, without claiming a confirmed regression or a dependency-constraint violation.

The problem is the practical result of automatic selection, not evidence that uv violated declared package requirements. The old torchvision wheel explicitly permits any torch version. The documentation describes automatic selection of the most-compatible PyTorch index for torch-family packages, while this report exposes a case where the selected index supplies a much older companion package.

The fallback fix in astral-sh/uv#12385 remains in the checkout. The general reprioritization fix in astral-sh/uv#9843 is threshold-based and predates cu129 support. Neither historical fix proves that this particular Windows scenario was previously resolved correctly. The open astral-sh/uv#17649 is a useful resolver analogue, but its build-failure scenario does not account for the separate automatic backend-priority symptom.

## Related

- astral-sh/uv#15416 — pull request, merged: add cuda 129 backend. Added cu129 ahead of cu128 and cu126 for Windows drivers >=528.33 on 2025-08-21. Its diff directly establishes the backend ordering involved here; it did not address incomplete Windows torch/torchvision release pairs.
- astral-sh/uv#12357 — issue, closed: Does auto option in ``torch-backend`` preview only select the highest supported CUDA version?. Earlier Windows auto-selection failure when the first CUDA index lacked wheels satisfying the Python and torch constraints. A maintainer confirmed that compatible indexes should be tried sequentially. Unlike this report, resolution failed instead of succeeding with legacy torchvision.
- astral-sh/uv#12385 — pull request, merged: Check all compatible torch indexes when `--torch-backend` is enabled. Fixed astral-sh/uv#12357 on 2025-03-22 by fetching across compatible PyTorch indexes using UnsafeFirstMatch. That override remains in the checkout, so this report does not demonstrate loss of the earlier fallback fix: an old candidate can satisfy metadata constraints in the preferred index.
- astral-sh/uv#17649 — issue, open: Resolver pins direct dependency instead of backtracking, causing Python-incompatible resolution. Closest open resolver analogue: retaining a recent direct dependency sends another package back to old releases with permissive metadata; maintainers recommend meaningful lower bounds. Its reproduction concerns numpy/numba, uv sync, and build-time Python incompatibility, rather than automatic CUDA index ordering and successful installation of legacy torchvision.
- astral-sh/uv#8157 — issue, closed: Tracking issue: uv backtracks on the wrong package. Canonical historical discussion of selecting a recent package A and backtracking package B until an old release drops the conflicting constraint. This closely explains the legacy-wheel symptom, but the tracker predates cu129 support and does not establish that this Windows backend case previously worked.
- astral-sh/uv#9843 — pull request, merged: Change backtracking when packages conflict too much. Closed astral-sh/uv#8157 on 2024-12-16 with conflict-count-based reprioritization. The PR explicitly describes old permissive dependencies yielding undesirable but metadata-valid resolutions. It is a heuristic improvement, not a guarantee against every legacy selection; no regression of its covered cases was established.

## Report decomposition

- Commands and subsystem: `uv pip install` and `uv pip compile`, automatic PyTorch backend selection, ordered CUDA indexes, and dependency candidate selection.
- Trigger: Windows target, CUDA 12.x-compatible driver, unbounded torch and torchvision requirements. Exact identifiers include `UV_CUDA_DRIVER_VERSION=560.94`, `WINDOWS_CUDA_VERSIONS`, `cu129`, and `x86_64-pc-windows-msvc`.
- Primary symptom: a recent torch is paired with torchvision 0.2.0, without a warning. Reported runtime effects are missing `InterpolationMode`, `efficientnet_b0`, and `torchvision.ops`.
- Separate symptom: torch itself is older than versions available from lower-priority, driver-compatible CUDA indexes.
- Proposed capabilities: prefer a better-populated backend, fall through when modern companion wheels are unavailable, or avoid legacy releases through version bounds. These are possible remedies, not an agreed implementation.
- Mechanism to verify separately: index ordering, per-index candidate preference, and permissive metadata on old wheels.

The reporter's cross-platform reproduction uses a requirements file containing `torch` and `torchvision`, with `UV_CUDA_DRIVER_VERSION=560.94 uv pip compile requirements.in --torch-backend=auto --python-platform x86_64-pc-windows-msvc`.

## Supporting evidence

### Current source

- `crates/uv-torch/src/backend.rs:386` enumerates driver-compatible Windows indexes in table order. At line 808, `WINDOWS_CUDA_VERSIONS` places cu132/cu130 behind a minimum driver of 580, then cu129, cu128, and cu126 at 528.33. Driver 560.94 therefore starts with cu129. The diff of astral-sh/uv#15416 confirms when cu129 was added ahead of the other CUDA 12 indexes.
- `crates/uv-client/src/registry_client.rs:327` overrides metadata fetching to `UnsafeFirstMatch` for PyTorch packages. At line 418, unsafe strategies fetch across indexes. Consequently, “uv only fetches the first index containing the package” is not an accurate description of this path.
- `crates/uv-resolver/src/candidate_selector.rs:524` merges versions across indexes for `UnsafeBestMatch`; the default branch at line 577 instead walks version maps in index order. This supports preference for an acceptable candidate in an earlier index over newer candidates elsewhere.
- `crates/uv-resolver/src/resolver/mod.rs:118` retains a conflict threshold of five for the broader backtracking heuristic. No resolver trace was collected to establish its exact role in this reproduction.
- `docs/guides/integration/pytorch.md:503` documents automatic backend selection and explicit backend overrides.
- The existing `incompatible_cuda` integration test in `crates/uv/tests/pip_compile/pip_compile.rs:19626` checks rejection of packages requiring different CUDA versions. It does not cover a permissive legacy torchvision wheel on an otherwise accepted backend. That test originated in astral-sh/uv#13179.

### Public index and wheel checks

Read-only checks on 2026-10-28 inspected the torch and torchvision listings under `https://download.pytorch.org/whl/` for cu129, cu128, and cu126. The following are the latest listed CPython 3.13 Windows x86-64 wheels, not an independently executed resolution:

| Index | torch | torchvision |
| --- | --- | --- |
| cu129 | 2.9.0+cu129 | 0.23.0+cu129 |
| cu128 | 2.11.0+cu128 | 0.26.0+cu128 |
| cu126 | 2.14.1+cu126 | 0.29.1+cu126 |

The cu129 torchvision listing also contains platform-independent 0.1.6 and 0.2.0 wheels. The inspected metadata for `torchvision-0.23.0+cu129-cp313-cp313-win_amd64.whl` requires `torch==2.8.0+cu129`. The 0.2.0 wheel metadata lists `numpy`, `pillow>=4.1.1`, `six`, and unbounded `torch`, with no Requires-Python field. Only metadata was read; no package code was executed.

These checks confirm the key asymmetry and permissive legacy dependency. They do not independently verify every Python version, the Linux comparison, runtime imports, or the reporter's complete resolution outputs.

### Historical status and excluded candidates

astral-sh/uv#12385 merged on 2025-03-22, astral-sh/uv#15416 on 2025-08-21, and astral-sh/uv#9843 on 2024-12-16. All predate the reported uv versions. No fix specifically addressing this Windows cu129/legacy-wheel combination was identified.

- astral-sh/uv#14742 concerns GPU compute capability, not stale package availability on an otherwise driver-compatible backend.
- astral-sh/uv#16386 concerns torchvision 0.24 CPU wheel local-version suffixes and failed universal installation. Its closing comment attributes resolution to corrected upstream 0.25 suffixes; it is not this CUDA-index case.
- astral-sh/uv#15298 concerns selecting platform-compatible files for the same version across indexes during universal resolution, including with unsafe-best-match. Here a valid wheel is selected and installation succeeds.
- astral-sh/uv#2562 concerns historical local-version handling on an explicitly chosen CPU index.
- astral-sh/uv#6141 requests warnings for very old packages selected because of newer Python requirements; it overlaps only with the missing-warning symptom.
- astral-sh/uv#16522 requests general wheel-variant selection beyond the existing PyTorch backend.
- astral-sh/uv#21162 requests a custom PyTorch index URL; it does not change backend preference or legacy candidate selection.

## Search coverage

Used authenticated gh to search open and closed issues with literal terms torch-backend, cu129, Windows, torchvision 0.2.0/0.1.6, InterpolationMode, and efficientnet_b0; separately searched outdated/latest selection, downgrades, old or ancient releases, minimum bounds, backtracking, and compatible-index fallback, including platform/package-independent queries and closed-fix searches. Supplemented PR search queries with a paginated all-state PR title inventory covering open, closed, and merged PRs, then inspected relevant bodies, comments, reviews, diffs, and issue timelines. Followed astral-sh/uv#12357 to astral-sh/uv#12385 and astral-sh/uv#1575 to astral-sh/uv#8157, astral-sh/uv#9843, and astral-sh/uv#17649. Ruled out astral-sh/uv#14742 (GPU compute capability), astral-sh/uv#16386 (CPU wheel local-version suffixes), astral-sh/uv#15298 (same-version platform fallback during universal resolution), and astral-sh/uv#2562 (historical local-version handling) as duplicates. Also inspected astral-sh/uv#6141 (warnings for Python-driven old-version fallback), astral-sh/uv#16522 (general wheel variants), and astral-sh/uv#13179 (CUDA consistency). Checked current source and public PyTorch wheel listings/metadata; did not run an end-to-end reproduction.

PR keyword searches did not return known matching PRs, so the review also enumerated all available PR titles with `gh pr list --state all --limit 20000`, filtered for PyTorch/CUDA and index-fallback terms, and inspected relevant results and linked history. Empty keyword-search results were not treated as evidence that no related PR existed.

## Next maintainer step

Add a focused reproduction using controlled index metadata: an earlier compatible CUDA index with a newer torch and only an older pinned torchvision pair plus a legacy unbounded wheel, and a later compatible index with a modern matched pair. Establish the intended backend/version preference before choosing an ordering change, resolver heuristic, or legacy-package policy. Keep the existing cross-CUDA consistency constraint covered.

The reporter already supplied driver, Python, uv versions, commands, and workarounds; no additional information is required for initial classification. An explicit cu128 backend is the reported immediate workaround. A meaningful torchvision lower bound protects application API requirements; `torchvision>=0.3` only excludes the two legacy releases. The reported unsafe-best-match workaround was not independently exercised.

## Scope of this handoff

Only this README was written. No checkout files, tests, issues, labels, comments, or pull requests were changed.
