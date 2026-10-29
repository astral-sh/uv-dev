# --torch-backend=auto on Windows with CUDA 12.x driver resolves torchvision 0.2.0 (2018)

Issue: astral-sh/uv#22965

Classification: bug

## Summary

The reporter says unpinned `uv pip install torch torchvision --torch-backend=auto` selects `torch 2.9.0+cu129` and `torchvision 0.2.0` on Windows with NVIDIA drivers >=528.33 and <580. Installation and basic imports succeed, but modern torchvision symbols and operations are missing. The report covers uv 0.13.6 and 0.12.16, Python 3.10–3.13, an RTX 3060 Ti, and simulated driver version 560.94. Linux CUDA 12.x and Windows CUDA 13 are reported unaffected.

astral-sh/uv#15416 introduced the relevant cu129 priority. astral-sh/uv#12357 and astral-sh/uv#12385 cover earlier fallback behavior; astral-sh/uv#17649, astral-sh/uv#8157, and astral-sh/uv#9843 cover pathological backtracking to permissive old releases. No exact duplicate or confirmed regression was found.

The reported resolution is independently reproduced on 2026-10-28 with the installed uv 0.13.5 on Linux, targeting Windows and simulating driver 560.94. Unpinned requirements resolve to `torch==2.9.0+cu129` and `torchvision==0.2.0`, with exit code 0 and no warning. Both reported platform/driver controls and all three reported workarounds also match. Windows installation, GPU execution, and runtime import failures were not tested. The parent regression from astral-sh/uv-dev#3002 also passes against a fresh debug build of the checkout (uv 0.13.6), asserting the undesirable pair. No fix was applied because the replacement selection policy requires a design decision.

## Draft response

Reproduced with uv 0.13.5 and Python 3.12, using `uv pip compile` with a Windows target and `UV_CUDA_DRIVER_VERSION=560.94`. The command succeeds without a warning and selects torch 2.9.0+cu129 with torchvision 0.2.0. An annotated run confirms that both packages come from the cu129 index.

The trace selects torch 2.9.0 first, tries torchvision 0.23.0+cu129 (which requires torch 2.8.0+cu129), then accepts torchvision 0.2.0 with an unbounded torch dependency. This satisfies declared package requirements; the reporter observes missing modern torchvision APIs at runtime. The earlier compatible-index fallback from astral-sh/uv#12385 remains present; this is not evidence that later indexes are never fetched.

The reported `--torch-backend=cu128`, `--index-strategy unsafe-best-match`, and `torchvision>=0.3` workarounds all produce the reported newer torchvision pairs in this reproduction. Applications should specify a torchvision lower bound appropriate for the APIs they require; `>=0.3` only excludes the legacy releases. The intended preference between backend priority and newer matched package pairs needs a maintainer decision before choosing a fix.

## Classification

Automatic backend selection succeeds with a severely obsolete torchvision despite newer matched Windows pairs on other driver-compatible indexes. The resolution, cu129 index selection, dependency metadata, and successful workaround resolutions are independently confirmed. The existing open resolver analogue does not cover automatic backend ordering, and the historical fallback and backtracking fixes do not establish that this specific case previously worked. Classify as a distinct bug, without claiming a confirmed regression or a dependency-constraint violation.

The problem is the practical result of automatic selection, not evidence that uv violated declared package requirements. The old torchvision wheel explicitly permits any torch version. The documentation describes automatic selection of the most-compatible PyTorch index for torch-family packages, while this report exposes a case where the selected index supplies a much older companion package.

The fallback fix in astral-sh/uv#12385 remains in the checkout. The general reprioritization fix in astral-sh/uv#9843 is threshold-based and predates cu129 support. Neither historical fix proves that this particular Windows scenario was previously resolved correctly. The open astral-sh/uv#17649 is a useful resolver analogue, but its build-failure scenario does not account for the separate automatic backend-priority symptom.

## Reproduction

**Outcome: reproducible.** Tested on 2026-10-28 with the installed `uv 0.13.5 (x86_64-unknown-linux-gnu)` at `/opt/hostedtoolcache/uv/0.13.5/x86_64/uv`, on Linux x86-64 with CPython 3.12.3 and `--python-version 3.12`. The Windows target is `x86_64-pc-windows-msvc`; the driver is simulated through `UV_CUDA_DRIVER_VERSION=560.94`. This exercises the report's cross-platform compile reproduction without requiring Windows or a GPU.

All reproduction inputs, caches, temporary files, and logs are under `/tmp/uv-22965-0kre2uvk`. Each subprocess used a clean environment containing only the inherited PATH and explicit temporary-storage/uv settings: `UV_NO_CONFIG=1`, `UV_PYTHON_DOWNLOADS=never`, `UV_KEYRING_PROVIDER=disabled`, temporary `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR`, `TMPDIR`, and XDG cache/config/data directories, plus the simulated driver. No project configuration, dependency pins, existing lockfile, or custom package indexes were used. Package metadata came from the public PyTorch indexes and PyPI.

Minimal commands, run in an isolated temporary directory with those settings:

```sh
printf 'torch\ntorchvision\n' > requirements.in
UV_CUDA_DRIVER_VERSION=560.94 uv pip compile requirements.in \
  --torch-backend=auto \
  --python-platform x86_64-pc-windows-msvc \
  --python-version 3.12
```

Observed: exit code 0, `Resolved 14 packages`, and no warning. The output includes:

```text
torch==2.9.0+cu129
    # via
    #   -r requirements.in
    #   torchvision
torchvision==0.2.0
    # via -r requirements.in
```

A repeat with `-v` and `--emit-index-annotation` reports both packages from `https://download.pytorch.org/whl/cu129`. Its trace confirms driver 560.94, selection of `torch-2.9.0+cu129-cp312-cp312-win_amd64.whl`, an attempted `torchvision==0.23.0+cu129` with `torch==2.8.0+cu129`, and subsequent selection of `torchvision-0.2.0-py2.py3-none-any.whl` with an unbounded `torch` dependency. This directly observes the legacy candidate selection; it does not establish why the general conflict-threshold heuristic did not reprioritize these packages.

Targeted controls and workarounds used the same Python target, environment, and unpinned requirements except for the stated change. All exited 0:

| Change from the Windows/560.94/auto baseline | Resolved torch | Resolved torchvision |
| --- | --- | --- |
| Baseline | 2.9.0+cu129 | 0.2.0 |
| Replace platform with `x86_64-unknown-linux-gnu` | 2.13.0+cu129 | 0.28.0+cu129 |
| Change simulated driver to `580` | 2.14.1+cu132 | 0.29.1+cu132 |
| Replace backend with `--torch-backend=cu128` | 2.11.0+cu128 | 0.26.0+cu128 |
| Add `--index-strategy unsafe-best-match` | 2.14.1+cu126 | 0.29.1+cu126 |
| Compile `bounded.in` containing `torch` and `torchvision>=0.3` | 2.8.0+cu129 | 0.23.0+cu129 |

Saved evidence: `baseline.stdout`/`.stderr`, `annotated.stdout`/`.stderr`, and corresponding `linux`, `driver580`, `cu128`, `best-match`, and `bounded` logs in the reproduction directory. Commands are recorded in the generated output headers and command JSON files; driver 580 was the sole environment change for that control.

**Existing test coverage:** searched `crates/uv/tests/` and `crates/uv-client/tests/it/` for automatic backend, driver override, and legacy torchvision coverage. Read the setup and assertions of `crates/uv/tests/pip_compile/pip_compile.rs::incompatible_cuda`: it pins torch 2.2.1+cu121 and torchvision 0.17.1+cu118, simulates driver 525.60.13, targets manylinux/Python 3.11 with auto backend, and expects resolution to fail on conflicting `system:cuda` dependencies. It does not cover successful selection of an unbounded legacy torchvision wheel or Windows cu129 priority. Its containing module is gated by `test-python` and `test-pypi` in `crates/uv/tests/pip_compile/main.rs`. The initial reproduction found no test for this exact selection. The checkout now includes `pip_compile::torch_backend_auto_windows_legacy_torchvision` from astral-sh/uv-dev#3002 in the same file and under the same module feature gates. It uses unpinned torch/torchvision, Python 3.12, Windows x86-64, driver 560.94, and an exclude-newer cutoff of 2026-10-27. Its snapshot asserts torch 2.9.0+cu129 and torchvision 0.2.0, including cu129 index annotations and a successful 14-package resolution. The fix investigation ran this test successfully without changing its snapshot.

**Limits:** the observed result establishes the reported dependency resolution, not Windows installation success, GPU compatibility, or the claimed runtime import errors. The initial command reproductions exercised installed uv 0.13.5 and Python 3.12. The fix investigation additionally exercised a debug build of the uv 0.13.6 checkout through the parent Python 3.12 regression test. Published uv 0.13.6/0.12.16 binaries and Python 3.10/3.11/3.13 combinations were not separately tested. There is no identified last-known-good version or confirmed regression. No additional reporter information is needed to reproduce the central selection behavior. Results use live public indexes and may change as their contents change.

## Related

- astral-sh/uv#14742 — adjacent GPU compatibility issue: automatic selection can choose a driver-compatible torch wheel that does not support an older GPU’s compute capability. This is distinct from the confirmed stale-package selection; the withdrawn Pascal/cu126 report does not establish a shared cause or an explicit-backend override defect.
- astral-sh/uv-dev#3002 — parent regression pull request: adds `torch_backend_auto_windows_legacy_torchvision` in `crates/uv/tests/pip_compile/pip_compile.rs`. The checkout contains its passing snapshot of the undesirable result. The test was verified during the fix investigation and remains unchanged.
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
- Confirmed mechanism in the compile trace: cu129 selection, a torchvision 0.23.0 torch-version conflict, and acceptance of the unbounded torch dependency in torchvision 0.2.0. The broader resolver heuristic remains a separate design question.

The reporter's cross-platform reproduction uses a requirements file containing `torch` and `torchvision`, with `UV_CUDA_DRIVER_VERSION=560.94 uv pip compile requirements.in --torch-backend=auto --python-platform x86_64-pc-windows-msvc`.

## Supporting evidence

### Current source

- `crates/uv-torch/src/backend.rs:386` enumerates driver-compatible Windows indexes in table order. At line 808, `WINDOWS_CUDA_VERSIONS` places cu132/cu130 behind a minimum driver of 580, then cu129, cu128, and cu126 at 528.33. Driver 560.94 therefore starts with cu129. The diff of astral-sh/uv#15416 confirms when cu129 was added ahead of the other CUDA 12 indexes.
- `crates/uv-client/src/registry_client.rs:327` overrides metadata fetching to `UnsafeFirstMatch` for PyTorch packages. At line 418, unsafe strategies fetch across indexes. Consequently, “uv only fetches the first index containing the package” is not an accurate description of this path.
- `crates/uv-resolver/src/candidate_selector.rs:524` merges versions across indexes for `UnsafeBestMatch`; the default branch at line 577 instead walks version maps in index order. This supports preference for an acceptable candidate in an earlier index over newer candidates elsewhere.
- `crates/uv-resolver/src/resolver/mod.rs:118` retains a conflict threshold of five for the broader backtracking heuristic. The collected resolver trace establishes candidate selection, but not the exact role of this threshold.
- `docs/guides/integration/pytorch.md:503` documents automatic backend selection and explicit backend overrides.
- The existing `incompatible_cuda` integration test in `crates/uv/tests/pip_compile/pip_compile.rs:19626` checks rejection of packages requiring different CUDA versions. It does not cover a permissive legacy torchvision wheel on an otherwise accepted backend. That test originated in astral-sh/uv#13179.

### Public index and wheel checks

Read-only checks on 2026-10-28 inspected the torch and torchvision listings under `https://download.pytorch.org/whl/` for cu129, cu128, and cu126. The following are the latest listed CPython 3.13 Windows x86-64 wheels from the earlier metadata inspection. The independent Python 3.12 resolutions are recorded above:

| Index | torch | torchvision |
| --- | --- | --- |
| cu129 | 2.9.0+cu129 | 0.23.0+cu129 |
| cu128 | 2.11.0+cu128 | 0.26.0+cu128 |
| cu126 | 2.14.1+cu126 | 0.29.1+cu126 |

The cu129 torchvision listing also contains platform-independent 0.1.6 and 0.2.0 wheels. The inspected metadata for `torchvision-0.23.0+cu129-cp313-cp313-win_amd64.whl` requires `torch==2.8.0+cu129`. The 0.2.0 wheel metadata lists `numpy`, `pillow>=4.1.1`, `six`, and unbounded `torch`, with no Requires-Python field. Only metadata was read; no package code was executed.

These metadata checks confirm the key asymmetry and permissive legacy dependency. The independent compile runs now also confirm the Windows baseline, Linux and driver-580 comparisons, and all three workaround pairs for Python 3.12. Other Python versions and runtime imports remain untested.

### Historical status and excluded candidates

astral-sh/uv#12385 merged on 2025-03-22, astral-sh/uv#15416 on 2025-08-21, and astral-sh/uv#9843 on 2024-12-16. All predate the reported uv versions. No fix specifically addressing this Windows cu129/legacy-wheel combination was identified.

- astral-sh/uv#14742 concerns GPU compute capability, not the original stale-package selection. The Pascal commenter has withdrawn the proposed connection to this issue, so that report provides no confirmed additional manifestation.
- astral-sh/uv#16386 concerns torchvision 0.24 CPU wheel local-version suffixes and failed universal installation. Its closing comment attributes resolution to corrected upstream 0.25 suffixes; it is not this CUDA-index case.
- astral-sh/uv#15298 concerns selecting platform-compatible files for the same version across indexes during universal resolution, including with unsafe-best-match. Here a valid wheel is selected, and installation is reported to succeed.
- astral-sh/uv#2562 concerns historical local-version handling on an explicitly chosen CPU index.
- astral-sh/uv#6141 requests warnings for very old packages selected because of newer Python requirements; it overlaps only with the missing-warning symptom.
- astral-sh/uv#16522 requests general wheel-variant selection beyond the existing PyTorch backend.
- astral-sh/uv#21162 requests a custom PyTorch index URL; it does not change backend preference or legacy candidate selection.

## Search coverage

The earlier context review used authenticated gh to search open and closed issues with literal terms torch-backend, cu129, Windows, torchvision 0.2.0/0.1.6, InterpolationMode, and efficientnet_b0; separately searched outdated/latest selection, downgrades, old or ancient releases, minimum bounds, backtracking, and compatible-index fallback, including platform/package-independent queries and closed-fix searches. Supplemented PR search queries with a paginated all-state PR title inventory covering open, closed, and merged PRs, then inspected relevant bodies, comments, reviews, diffs, and issue timelines. Followed astral-sh/uv#12357 to astral-sh/uv#12385 and astral-sh/uv#1575 to astral-sh/uv#8157, astral-sh/uv#9843, and astral-sh/uv#17649. Ruled out astral-sh/uv#14742 (GPU compute capability), astral-sh/uv#16386 (CPU wheel local-version suffixes), astral-sh/uv#15298 (same-version platform fallback during universal resolution), and astral-sh/uv#2562 (historical local-version handling) as duplicates. Also inspected astral-sh/uv#6141 (warnings for Python-driven old-version fallback), astral-sh/uv#16522 (general wheel variants), and astral-sh/uv#13179 (CUDA consistency). The earlier context review checked source and public PyTorch wheel listings/metadata. This reproduction pass additionally ran the cross-platform compile baseline, an annotated debug run, two platform/driver controls, and three workarounds using the installed uv executable; it made no GitHub requests or changes.

PR keyword searches did not return known matching PRs, so the review also enumerated all available PR titles with `gh pr list --state all --limit 20000`, filtered for PyTorch/CUDA and index-fallback terms, and inspected relevant results and linked history. Empty keyword-search results were not treated as evidence that no related PR existed.

## Fix

**Outcome: not_fixed.** The checkout is unchanged. The confirmed cause is the combination of Windows CUDA index priority, per-index candidate preference, and permissive upstream dependency metadata. The first compatible backend for driver 560.94 is cu129. Its torch 2.9.0 candidate conflicts with torchvision 0.23.0's torch 2.8.0 pin, while torchvision 0.2.0 accepts that torch version. The existing resolution is valid under the declared requirements; no lost constraint, incorrect platform mapping, or missing index fetch was established.

The implementation confirms that `TorchStrategy::index_urls` in `crates/uv-torch/src/backend.rs` iterates the Windows driver table in order. `RegistryClient::index_strategy_for` in `crates/uv-client/src/registry_client.rs` fetches all compatible torch indexes using `UnsafeFirstMatch`. `CandidateSelector::select_no_preference_from` in `crates/uv-resolver/src/candidate_selector.rs` prefers earlier version maps unless `UnsafeBestMatch` is requested. The resolver's system-dependency path associates these cu129 candidates with the same CUDA version, so the pair does not violate cross-CUDA consistency.

A correction requires choosing among materially different policies. Reordering or dropping cu129 changes automatic backend selection for all Windows CUDA 12.x torch-family requests, including those without torchvision. Imposing a torchvision floor excludes releases that users can currently request. Preferring versions globally or changing backtracking changes resolution semantics beyond this index's incomplete Windows release pair. The reproduction and parent test establish the undesirable result but do not specify which replacement pair or policy is intended. No speculative production change or arbitrary desired snapshot was introduced; the parent test was neither weakened nor removed.

**Nearby paths and coverage:** inspected the neighboring `incompatible_cuda` test, which correctly rejects mixed CUDA dependencies, and `pip_install::prerelease_index_strategy_ordering` in `crates/uv/tests/pip_install/pip_install.rs`, which explicitly distinguishes earlier-index preference from global best-match selection. Their assertions are not contradicted by this cause. Inspected `TorchStrategy::from_mode` and the backend/client setup in `crates/uv-pip-commands/src/{compile,install,sync}.rs`: they use the same backend selection and registry implementation. No distinct producer/consumer defect or additional demonstrated manifestation justified another integration test. Explicit backend selection and unsafe-best-match remain the separately verified workarounds described above.

**Successful focused validation:**

```sh
cargo test --locked -p uv --test pip_compile --features test-python,test-pypi \
  pip_compile::torch_backend_auto_windows_legacy_torchvision -- --exact --nocapture
```

The debug/test-profile build succeeded. The exact parent test passed: 1 passed, 0 failed, 414 filtered out. Its command exited 0 and resolved torch 2.9.0+cu129 with torchvision 0.2.0 from cu129, without warnings. This validates the baseline undesirable behavior, not a fix. No desired-behavior failure was claimed because no replacement policy was selected. No full suite, release build, or Windows compilation was run, and no Rust formatting was needed because no Rust files changed.

## Withdrawn configured-backend report

The commenter who reported a Pascal GPU and an apparently overridden `cu126` setting now says their issue may be unrelated and that they are deleting the earlier post. The supplied issue discussion no longer contains that post. Source: follow-up comment ID 6501244768, dated 2026-10-29, supplied separately in `issue-comment.json`. Treat the earlier report as a withdrawn association, not evidence of an additional affected platform, a settings-precedence regression, or a failure of the verified workarounds. The underlying settings problem was not reproduced or explained; the follow-up does not establish that it is resolved or definitively unrelated. No separate settings investigation is an active next step for astral-sh/uv#22965 on this evidence.

The source finding from that review remains valid: in `crates/uv-torch/src/backend.rs`, `TorchStrategy::from_mode` maps an effective `TorchMode::Cu126` directly to `TorchBackend::Cu126` and returns `Self::Backend` (lines 254 and 299), whose `index_urls` branch yields only that backend’s index (line 462). Automatic index ordering therefore does not replace an effective explicit cu126 selection. This does not establish which settings were effective in the withdrawn report. The confirmed automatic-selection reproduction and classification are unchanged.

## Next maintainer step

Choose whether automatic selection should prioritize a different Windows CUDA backend, a minimum torchvision release, or a newer matched pair across indexes. Then update the retained parent regression to that agreed expectation and demonstrate failure before implementing the corresponding focused change. Keep `incompatible_cuda` and intentional index-strategy ordering intact. A controlled index fixture may help stabilize the eventual regression against future upstream wheel additions.

No additional reporter information is needed to reproduce the central selection behavior. Explicit cu128, unsafe-best-match, and the torchvision lower bound were independently exercised successfully in the reproduction. A meaningful torchvision lower bound protects application API requirements; `torchvision>=0.3` only excludes the two legacy releases. These compile results do not establish runtime GPU compatibility. Preserve explicit backend selection when choosing an automatic-selection policy; the withdrawn configured-cu126 report does not add a separate investigation requirement.

## Scope of this handoff

The investigation updated only this README among the issue-context files. Earlier command reproductions used `/tmp/uv-22965-0kre2uvk`; the focused debug test generated build artifacts and temporary test files. No checkout source, regression tests, manifests, lockfiles, workflows, Git configuration, issues, labels, comments, or pull requests were changed. No commits or pushes were made.
