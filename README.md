# Universal Windows lock cannot sync on free-threaded Python 3.13

Issue: astral-sh/uv#23239

Classification: duplicate

## Summary

The reporter uses uv 0.13.8 on Ubuntu 26.04.1 under WSL2 to create a universal lock restricted to Windows, with both environments and required-environments set to sys_platform == 'win32'. The project requires Python >=3.13,<3.14 and depends on Pympler.

The reported commands are:

```console
uv lock --python 3.13
uv sync --locked --dry-run --python 3.13t --python-platform x86_64-pc-windows-msvc
```

Locking selects Pympler 1.1 and pywin32 312. Sync selects CPython 3.13.12+freethreaded but rejects pywin32 because it has no source distribution or compatible wheel. Locking with --python 3.13t and using fork-strategy="fewest" reportedly make no difference. The reporter says Pympler 1.0.1 works with both interpreter builds.

astral-sh/uv#9711 is the canonical discussion. astral-sh/uv#9425 covers a similar interpreter-specific wheel gap; merged astral-sh/uv#10046 fixed only a narrower local-version/platform case.

## Impact and cross-project context

The reporter clarifies that this arose while building a single development/CI lockfile for `packaging`, so it affects a real shared-lock workflow rather than only the minimal example. They personally consider it low priority; this is the reporter's assessment, not a maintainer priority decision.

The reporter also supplied parallel Poetry and PDM reports and described a nab pull request as a partial workaround. These are investigation leads, not independently verified fixes or evidence that the tools have identical mechanisms. Their contents and the applicability of the nab approach to uv have not been reviewed in this update. No new uv reproduction steps or verified uv workaround were supplied.

Source: reporter comment at https://github.com/astral-sh/uv/issues/23239#issuecomment-6861195226.

## Draft response

This is another instance of the incomplete universal-resolution problem tracked in astral-sh/uv#9711. Pympler 1.1 requires pywin32 on Windows, and pywin32 312 publishes neither a source distribution nor a Python 3.13t wheel. The current environment markers cannot distinguish GIL and free-threaded CPython, so required-environments cannot express that additional requirement. Please continue the discussion in astral-sh/uv#9711 with this reproduction. For now, your pympler==1.0.1 pin avoids that Windows pywin32 dependency.

## Classification

The lock-to-sync correctness failure is the incomplete universal-resolution problem already tracked by open astral-sh/uv#9711. Public metadata confirms that Pympler 1.1 adds a Windows-only pywin32 dependency and pywin32 312 has no source distribution or 3.13t wheel; the checkout's marker model cannot express the GIL/free-threaded distinction. The more specific trigger can be centralized in that tracking issue. No evidence shows that a previous fix covered this case and regressed.

The underlying behavior is a correctness problem: successful universal resolution does not ensure that the selected wheel-only dependency can be installed in the intended environment. Duplicate takes precedence because the open tracking issue already covers that failure and the need to backtrack to a compatible dependency set. The missing marker explains a constraint on solutions; it does not make the failed lock-to-sync workflow correct.

The historical local-version fix predates this report but never claimed to solve free-threaded compatibility. There is no evidence that this is a recurrence of a previously fixed case, or that a pull request created for this new report establishes duplication.

## Related

- astral-sh/uv#9711 — Issue, open: Tracking issue: package without source distribution gets an "incomplete" resolution. Canonical tracking issue for universal resolution accepting a wheel-only release that covers some environments, then sync failing with the same missing-distribution error elsewhere. This report adds Windows CPython 3.13 GIL/free-threaded coverage, which the current environment markers cannot distinguish.
- astral-sh/uv#9425 — Issue, open: `uv add onnxruntime` fails to fetch latest version of package compatible with my environment's Python. Closely related Python compatibility case: universal resolution selects a wheel-only release that cannot install on the requested interpreter instead of backtracking. Maintainers recommend a python_version required environment there; that workaround cannot distinguish 3.13 from 3.13t in this report.
- astral-sh/uv#10046 — Pull request, merged: Backtrack to non-local versions when wheels are missing platform support. Partial fix referenced by astral-sh/uv#9711, merged on 2024-12-20. It only considers replacing local versions such as +cpu with their non-local base version when platform wheels are missing. It does not cover ordinary Pympler/pywin32 versions or free-threaded ABI coverage, so it does not establish a regression.

- python-poetry/poetry#11228 — Reporter-provided parallel Poetry issue for this use case; contents and relationship beyond the reporter's attribution have not been independently reviewed.
- pdm-project/pdm#4006 — Reporter-provided parallel PDM issue for this use case; contents and relationship beyond the reporter's attribution have not been independently reviewed.
- notatallshaw/nab#1576 — Reporter-provided pull request described as a partial workaround in nab. Its implementation, status, and applicability to uv have not been independently verified.

## Supporting evidence

### Published artifacts

Public PyPI release metadata was inspected without downloading or executing package code:

- https://pypi.org/pypi/Pympler/1.1/json declares pywin32>=226; platform_system == "Windows".
- https://pypi.org/pypi/Pympler/1.0.1/json declares no Requires-Dist dependencies. This supports the reporter's workaround: that release avoids introducing pywin32 through this dependency edge.
- https://pypi.org/pypi/pywin32/312/json lists no source distribution. Its Python 3.13 wheels are pywin32-312-cp313-cp313-win32.whl, pywin32-312-cp313-cp313-win_amd64.whl, and pywin32-312-cp313-cp313-win_arm64.whl. None has the cp313t ABI tag.

These artifact facts support the reported installation failure. The reproduction and runtime compatibility of Pympler 1.0.1 were not independently tested.

### Repository source and documentation

- docs/concepts/resolution.md:190 documents the default requirement of at least one wheel compatible with the target Python version and the purpose of required-environments. The setting accepts PEP 508 markers.
- crates/uv-pep508/src/marker/environment.rs:17 lists the interpreter marker fields. It has Python version, implementation, OS, and platform values, with no free-threaded/GIL field.
- crates/uv-resolver/src/prioritized_distribution.rs:1188 derives Python coverage from wheel tags as version and implementation markers. It cannot encode the CPython 3.13 GIL/free-threaded distinction in that marker model.
- crates/uv-resolver/src/version_map.rs:851 distinguishes concrete tag compatibility checks from the universal path's Python requirement check.
- crates/uv-platform-tags/src/tags.rs:177 records free-threading as a CPython ABI variant when generating concrete interpreter tags. Recognizing that ABI at installation time is separate from expressing it as a universal resolution marker.

These are findings from the current checkout, consistent with the report; they are not an executed reproduction of uv 0.13.8.

### Maintainer discussion and fix history

In astral-sh/uv#9711, the original report describes the same error and explains that a wheel-only version can be accepted despite incomplete environment coverage. Maintainers discuss determining coverage and backtracking, and later identify required-environments as an available mitigation.

In astral-sh/uv#9425, the reported gap is between Python minor versions. A maintainer recommends required-environments = ["python_version == '3.9'"]. The important difference here is that 3.13 and 3.13t share the version marker, so that mitigation cannot express the additional ABI requirement.

astral-sh/uv#9928 proposed broader platform-based forking but was closed without merging. A maintainer's December 20, 2024 comment says astral-sh/uv#10046 was merged instead. The merged pull request explicitly limits its behavior to local-version alternatives, leaving the general tracking issue open.

## Search decomposition and coverage

The report was separated into these claims before searching:

- Observable failure: uv lock succeeds, then uv sync --locked --dry-run cannot install a selected dependency.
- Trigger: Windows AMD64 and free-threaded CPython 3.13, with a Python range limited to 3.13.
- Required coverage: environments and required-environments both select Windows, but do not ensure the intended interpreter ABI.
- Reported capability gap: markers cannot choose dependencies based on free-threadedness.
- Exact identifiers and variants: Pympler 1.1 versus 1.0.1, pywin32 312, 3.13t, --python-platform, and fork-strategy="fewest".

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered Pympler, pywin32, pywin32==312, 3.13t, cp313t, required-environments, and the exact missing-source-or-wheel error. Separate conceptual searches covered free-threaded/free-threading/free threading, GIL/no-GIL, markers, universal locking, incomplete resolution, wheel-only dependencies, ABI coverage, and backtracking. Inspected candidate bodies and comments and followed astral-sh/uv#9711 to closed unmerged astral-sh/uv#9928 and merged astral-sh/uv#10046. Ruled out astral-sh/uv#19508 as a mid-upload wheel snapshot problem; astral-sh/uv#14040 and merged astral-sh/uv#14041 as a Windows architecture-marker fix; astral-sh/uv#16434 as interpreter discovery; and astral-sh/uv#2429 as incorrect installation of incompatible ABI wheels. Also inspected astral-sh/uv#12721, astral-sh/uv#11855, and marker/support discussions. Some auxiliary REST searches were rate-limited. Checked repository marker/tag handling and public PyPI release metadata; did not execute the reproduction.

Other inspected candidates were less suitable for centralizing this report:

- astral-sh/uv#19508 has a superficially close GIL/free-threaded wheel error, but its established trigger is locking during an incomplete upstream upload. The maintainer points to PEP 694. This report has a persistently absent artifact.
- astral-sh/uv#14040 and its merged fix astral-sh/uv#14041 concern incorrect AMD64 marker inference and reject resolution despite compatible Windows wheels. This report resolves successfully and then fails because the required ABI wheel is absent.
- astral-sh/uv#16434 concerns interpreter discovery rejecting 3.13t. Here the requested free-threaded interpreter is selected successfully.
- astral-sh/uv#2429 concerns installing incompatible ABI wheels and possible crashes. The new report rejects the incompatible dependency at sync time.
- astral-sh/uv#12721 includes macOS deployment-version and multi-index cases. astral-sh/uv#11855 requests making the current platform a default required environment. Neither specifically addresses distinguishing free-threaded ABI coverage when required-environments is already configured.
- astral-sh/uv#17382 concerns documenting supported marker values, and astral-sh/uv#15756 concerns interpreter-selection support and documentation. They do not track this lock-to-sync failure.

## Handoff status

The README and existing structured result agree on duplicate classification and astral-sh/uv#9711 as the canonical discussion. The reporter-provided workflow, priority assessment, and cross-project leads above supplement the original triage; the structured artifacts were left unchanged. The proposed reply is for maintainer review only. No GitHub changes were made, no reproduction commands or builds were run, and no checkout files were modified.
