# `uv lock --upgrade-package` fails with "No solution found" on a lock the same command just wrote

Issue: astral-sh/uv#23142

## Summary

Reproduced on Linux x86_64 with CPython 3.12.3 and the installed uv 0.13.6 (GNU build). Using the reported paperless-ngx revision, the first package-specific upgrade succeeds and updates torch from 2.14.0 to 2.14.1. `uv lock --python 3.12 --locked` accepts that lock without changing it. Repeating the same package-specific upgrade fails on the reported Python >=3.15 / PyPy / macOS split with a torch wheel-availability error.

A reduced project containing torch and a `dev` dependency on daphne also reproduces the successful-upgrade / successful-locked-check / failed-repeat sequence. Its failing split starts at Python 3.12 rather than 3.15. Git dependencies are therefore not necessary to reproduce the repeated-upgrade failure, although retaining Git pins is the reporter's reason for using package-specific upgrades.

Plain locking accepts the full project's first-pass lock unchanged. Two full upgrades also succeed, but the first moves its Git dependency, confirming that full upgrade changes the intended workflow. No root cause or fix has been confirmed.

## Classification

Bug report; the inconsistent repeated-resolution behavior is confirmed. Acceptance by `--locked` validates the existing lock against the project and does not establish that a fresh resolution or installation on every represented platform succeeds. The evidence does not determine whether the first solve should reject the unsupported environment or the repeated solve should retain the first solve's selection.

The reporter's description that the second solve “loses the fallback” remains a hypothesis. Source inspection identifies different treatment of stored forks for full and package-specific upgrades, but does not establish the responsible constraint or fallback. Duplicate classification and recurrence of a previously fixed bug are not established.

## Reproduction

**Outcome: reproducible.** These are observed command results, not conclusions inferred from related issues or source code.

### Environment and isolation

- Reported: uv 0.12.15, `uv-x86_64-unknown-linux-musl`, Linux x86_64, CPython 3.12.
- Tested: installed `uv 0.13.6 (x86_64-unknown-linux-gnu)` at `/opt/hostedtoolcache/uv/0.13.6/x86_64/uv`; CPython 3.12.3 at `/usr/bin/python3.12`; Linux x86_64 / Ubuntu 24.04.
- All project files, cache, configuration directories, temporary build files, and command artifacts were placed under `/tmp/uv-23142-ZVY4aR`. Commands ran with an explicit environment and isolated uv/XDG paths; managed Python downloads and Git global/system configuration were disabled. The successful reproduction used normal source-build behavior, not `--no-build`.
- The reported uv 0.12.15 build and checkout's uv 0.13.7 were not executed. There is no reported last known-good uv release to compare. No uv binary was built from this checkout.

### Full reported project

Fetched only `pyproject.toml` and `uv.lock` from paperless-ngx/paperless-ngx revision `5f17b1a310aa011f43b56474221d97f8d6bfe689`, reviewed their resolver inputs, and used temporary copies. A full project clone was unnecessary because the root is a virtual project (`package = false`) with static dependency declarations.

Relevant configuration includes:

- `requires-python = ">=3.11"`, `prerelease = "disallow"`, and supported environments `sys_platform == 'darwin'` and `sys_platform == 'linux'`.
- `torch>=2.13,<2.15`, explicitly sourced from `pytorch-cpu` at `https://download.pytorch.org/whl/cpu`.
- Extras `mariadb`, `postgres`, and `webserver`; `dev` includes `docs`, `lint`, and `testing`; a separate `typing` group contains the Git dependency. These declarations were retained, including the default `dev` group. This is a root project, not a workspace-member selection scenario.
- Platform/Python-specific direct wheel sources for psycopg-c, and a locked Git dependency on microsoft/python-type-stubs at `692c37c3969d22612b295ddf7e7af5907204a386`.
- The original lock has 274 package records and ten resolution markers. The upgrade list contains 268 unique names with no Git source, including the virtual root and non-Git URL-source packages, matching the reporter's extraction rule.

Starting from the original two files, the command sequence was reconstructed using argument arrays rather than executing the issue's shell script:

```python
import subprocess
import tomllib
from pathlib import Path

lock = tomllib.loads(Path("uv.lock").read_text())
names = sorted({
    package["name"] for package in lock["package"]
    if "git" not in package.get("source", {})
})
upgrade_arguments = [
    argument for name in names
    for argument in ("--upgrade-package", name)
]
subprocess.run(["uv", "lock", "--python", "3.12", *upgrade_arguments], check=True)
subprocess.run(["uv", "lock", "--python", "3.12", "--locked"], check=True)
subprocess.run(["uv", "lock", "--python", "3.12", *upgrade_arguments], check=False)
```

| Invocation | Observed result |
| --- | --- |
| First package-specific upgrade | Exit 0; 283 package records; torch 2.14.0 / 2.14.0+cpu become 2.14.1 / 2.14.1+cpu; ten stored forks become twenty, introducing PyPy splits. |
| `uv lock --python 3.12 --locked` | Exit 0; first-pass lock is byte-identical. |
| Same package-specific upgrade, same argument list | Exit 1; the exact reported Python >=3.15 / PyPy / Darwin split fails. The lock remains unchanged. |
| Plain `uv lock --python 3.12` on first-pass lock | Exit 0; lock remains byte-identical. |
| `uv lock --python 3.12 --upgrade`, in a separate copy of first-pass state | Exit 0; Git pin advances from `692c37c3` to `19affca5`. |
| Repeated full upgrade in that separate copy | Exit 0. |

The failing output is:

```text
error: No solution found when resolving dependencies for split (markers: python_full_version >= '3.15' and platform_python_implementation == 'PyPy' and sys_platform == 'darwin')
  cause: Because torch>=2.13.0,<=2.14.1+cpu has no macOS-compatible wheels and only the following versions of torch are available:
             torch<=2.14.1+cpu
             torch>=2.15
         we can conclude that torch>=2.13.0,<2.15 cannot be used.
         And because your project depends on torch>=2.13,<2.15 and your project requires paperless-ngx[mariadb], we can conclude that your project's requirements are unsatisfiable.
```

The package-specific first pass retains the original Git revision. Its lock is saved at `/tmp/uv-23142-ZVY4aR/current-normal-pass1.lock` (SHA256 `06dcc694bd28a23476189cfd861fc016e902019988042205bc3dab02a3411a45`). The original inputs are `/tmp/uv-23142-ZVY4aR/pyproject.toml` and `/tmp/uv-23142-ZVY4aR/uv.lock`; the fixed upgrade argument list is `upgrade-arguments.json`; the failing output is `current-normal-pass2.log` in the same directory.

### Reduced reproduction

This temporary `pyproject.toml` reproduces the same sequence without Git sources, direct wheel URLs, extras, or the other paperless dependencies:

```toml
[project]
name = "repro"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = ["torch>=2.13,<2.15"]

[dependency-groups]
dev = ["daphne==4.2.3"]

[tool.uv]
package = false
prerelease = "disallow"
environments = ["sys_platform == 'darwin'", "sys_platform == 'linux'"]

[tool.uv.sources]
torch = { index = "pytorch-cpu" }

[[tool.uv.index]]
name = "pytorch-cpu"
url = "https://download.pytorch.org/whl/cpu"
explicit = true
```

Run these explicit commands in a fresh temporary directory containing that file:

```sh
# Seed an older lock; these version restrictions are CLI inputs only.
uv lock --python 3.12 --upgrade-package torch==2.14.0 --upgrade-package autobahn==26.7.1
# Exit 0: 35 package records.

uv lock --python 3.12 --upgrade-package torch --upgrade-package autobahn --upgrade-package cbor2
# Exit 0: 36 records; upgrades torch and autobahn, and selects two cbor2 versions.

uv lock --python 3.12 --locked
# Exit 0, byte-identical lock.

uv lock --python 3.12 --upgrade-package torch --upgrade-package autobahn --upgrade-package cbor2
# Exit 1: torch has no macOS-compatible wheels in the Python >=3.12 / PyPy / Darwin split.
```

The first upgrade selects autobahn 26.11.1 and cbor2 5.9.0 / 6.2.1; autobahn's cbor2 dependencies differ across `platform_python_implementation == 'PyPy'`. This provides a concrete dependency-group fixture for investigating the additional forks, without establishing a root cause. The reduced fixture is `/tmp/uv-23142-ZVY4aR/group-project/pyproject.toml`; its seed and successful first-upgrade locks are `group-seed.lock` and `group-pass1.lock`, with `group-pass2.log` containing the failure. The failed repeat leaves its first-pass lock unchanged.

A torch/mysqlclient-only reduction succeeded on both passes, so the error mentioning the mariadb extra does not establish that mysqlclient is necessary. Making autobahn a direct dependency instead of reaching it through daphne produced the wheel error on the first upgrade rather than reproducing the precise successful-first-pass sequence. Retain the demonstrated group fixture when reducing further. An initial full-project attempt with `--no-build` stopped at psycopg-c on its first pass; that separate build-policy failure is not evidence for this issue.

### Existing test coverage

Searched the integration tests under `crates/uv/tests/` (including the current `lock/`, `project/`, `lock_scenarios/`, and `it/` modules) and `crates/uv-client/tests/it/`, then read the relevant setup and assertions:

- `crates/uv/tests/lock/lock.rs::lock_upgrade_package` covers selectively upgrading anyio while retaining idna, subsequent successful `--locked` validation, and a full upgrade. It does not repeat a package-specific upgrade across wheel/fork boundaries.
- `crates/uv/tests/lock/lock.rs::lock_pytorch_local_preference` snapshots torch 2.6.0 for Darwin and 2.6.0+cpu elsewhere, using a dated PyTorch mirror and predeclared platform environments. It performs one lock operation, without a PyPy split or repeated upgrade.
- `crates/uv/tests/lock/lock.rs::lock_upgrade_drop_fork_markers` verifies that full `--upgrade` drops obsolete forks after relaxing a dependency bound. It does not exercise the reported repeated package-specific path.
- `crates/uv/tests/lock/lock.rs::lock_required_environment_python_fork` verifies per-Python-fork wheel coverage with `required-environments` and a stable-ABI fallback. The reported project does not configure `required-environments`.

These four tests are gated by `test-universal`. None of these inspected tests covers the exact reproduced sequence. No repository tests were added or executed; verification used the installed uv executable.

## Related

- astral-sh/uv#22040 — Open issue about varying resolution-marker order. It includes the same two-pass package-specific workflow with Git pins on uv 0.12.15, but its runs succeed. It explains the reporter's reason for a second pass, not this observed resolution failure.
- astral-sh/uv#6176 — Open issue about applying fork markers at fork time. Maintainers discuss differences between discovering fork points during resolution and starting with existing forks. Relevant context, without a confirmed shared mechanism.
- astral-sh/uv#17747 — Open package-specific-upgrade issue involving new forks and unstable output. Its subsequent `--locked` rejects the generated lock, unlike this reproduced sequence.
- astral-sh/uv#17752 — Open, unmerged marker-canonicalization experiment linked from astral-sh/uv#17747. A maintainer said it could not ship as-is. No demonstrated fix for this issue.
- astral-sh/uv#21672 — Merged change scoping required-environment checks to the current fork, released in uv 0.12.14 before the reported version. Relevant to wheel/fork checks, without a confirmed causal connection.
- astral-sh/uv#21825 — Merged Darwin-baseline wheel-boundary fix, released in uv 0.12.17. It does not establish a fix for the repeated-upgrade failure, which still occurs on tested uv 0.13.6.
- astral-sh/uv#21820 and astral-sh/uv#21835 — ONNX Runtime wheel selection across Python forks and its merged fix; astral-sh/uv#21836 also corrects generic/stable-ABI wheel handling. The fixes shipped in uv 0.12.18. Their successful-resolution/installation mismatch differs from this second-resolution failure.

Earlier research also considered the following alternatives without establishing a duplicate or fix:

- astral-sh/uv#5180 and merged astral-sh/uv#5480, astral-sh/uv#5481, astral-sh/uv#5485: historical fork persistence and second-run version collapse.
- astral-sh/uv#20368 and merged astral-sh/uv#20369: impossible cross-variable platform-marker memoization and alternating dependencies.
- astral-sh/uv#9963 and merged astral-sh/uv#10046: PyTorch local-version/non-local fallback. Related functionality, not proof of the cause here.
- astral-sh/uv#23073: invalid locks accepted after manual edits or merges. Here uv itself writes the lock and no manual edit is needed.
- astral-sh/uv#22805 and merged astral-sh/uv#22806: Android wheel-tag recognition, distinct from the Darwin split.
- astral-sh/uv#4987 and astral-sh/uv#13373: corrected private-index/workspace-source configuration, without an established connection.

## Implementation context and next steps

Release notes and the current implementation were inspected alongside the reproduction. In `crates/uv-lock-operations/src/validated_lock.rs`, full upgrade makes the existing lock unusable, while package-specific upgrade can retain it as preferable after fork-coverage checks. In `crates/uv-lock-operations/src/lock.rs`, satisfying/preferable locks supply initial universal fork markers. `crates/uv-resolver/src/resolver/mod.rs::fork_version_registry` handles local-version/non-local platform fallback and required-environment artifact checks. This distinction supports comparing fresh and reused forks, but does not confirm the faulty step.

Use the saved successful first-pass lock and the reduced daphne/dev fixture to trace torch selection before and after introducing or reusing the PyPy fork. A future regression test should retain the successful first upgrade, successful `--locked`, and repeated identical package-specific upgrade, and use the relevant `test-universal` gate. Full upgrade is a diagnostic comparison because it also advances Git sources. The existing information is sufficient to reproduce; requesting the reporter's first-pass lock is no longer a prerequisite for investigation.

## Verification and scope

The outcome is based on executed commands and saved lockfiles, supplemented by source, release-note, and test inspection. Earlier issue/PR research is retained as related context and does not establish a root cause. No fix was implemented or tested. Only temporary reproduction files and this issue-context README were written. No checkout files or GitHub state were changed; existing checkout modifications were left untouched.
