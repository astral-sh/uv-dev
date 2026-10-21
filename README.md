# existing package is not found

Issue: astral-sh/uv#22716

Classification: duplicate

## Summary

The reporter uses uv 0.13.0 with Python 3.12 on Windows 11 x64. Running `uv add PyQt5` selects `pyqt5-qt5==5.15.19` and fails because that dependency has neither a Windows wheel nor a source distribution. The reporter points to the Windows wheel for `PyQt5==5.15.11`; that is a different distribution.

Duplicate of astral-sh/uv#9711. astral-sh/uv#13299 confirms the same Windows/PyQt5 failure and required-environments workaround; astral-sh/uv#9907 establishes the canonical discussion. Historical PRs did not ship a general fix for this case.

Public PyPI metadata confirms the distinction. This is an existing universal-resolution problem involving a package whose platform coverage varies between releases. The report does not show an ABI-tag rejection, an ignored explicit version pin, or a failure after configuring required environments.

The reporter says manually downloading a wheel from the PyQt5 PyPI files page and installing it works. The exact wheel, installation command, target environment, and resulting dependency versions were not supplied; this is a reported workaround, not a verified reproduction or evidence that uv failed to discover the PyQt5 wheel.

## Reporter disposition

After the explanation of the separate PyQt5-Qt5 dependency, the reporter agrees that astral-sh/uv#22716 can be closed. This is the reporter's stated disposition, not a maintainer decision or confirmation of the issue's actual closed state; the supplied issue snapshot does not include a state field. It does not establish that `tool.uv.required-environments` was tried, that the original `uv add PyQt5` command now succeeds, or that the underlying resolver behavior was fixed. The duplicate classification and existing technical findings remain applicable.

## Draft response

The Windows wheel you found is for `PyQt5`; the error concerns its dependency `PyQt5-Qt5==5.15.19`. PyPI lists only Linux/macOS wheels and no source distribution for that dependency version.

This is tracked in astral-sh/uv#9711. The same Windows/PyQt5 case in astral-sh/uv#13299 was resolved by declaring Windows as a required environment. Add this to your `pyproject.toml`, merging it into any existing `[tool.uv]` section, then rerun `uv add PyQt5`:

```toml
[tool.uv]
required-environments = ["sys_platform == 'win32' and platform_machine == 'AMD64'"]
```

This tells uv to require Windows-compatible wheels during resolution. Please continue discussion in astral-sh/uv#9711.

## Classification

The same universal-resolution problem is already tracked in open astral-sh/uv#9711, including explicit maintainer discussion of PyQt5-Qt5 on Windows. PyPI confirms that PyQt5 5.15.11 has a Windows wheel, while its dependency PyQt5-Qt5 5.15.19 has neither Windows wheels nor a source distribution. Earlier matching reports establish the required-environments workaround. No regression is established: the general fix was not merged, and the merged local-version fix does not cover these releases.

Duplicate classification takes precedence because the open tracking issue already covers the underlying behavior. Its maintainer discussion specifically categorizes the PyQt5 Windows case, and astral-sh/uv#9907 was explicitly centralized there. The documentation and available workaround explain the behavior; they are not evidence that a previously shipped general fix has regressed.

The new report supplies no project configuration or lockfile and does not say that the recommended setting was already tried. There is no evidence for classifying it as the separate pin/marker-selection failure alleged in astral-sh/uv#19308.

## Related

- astral-sh/uv#9711 — Tracking issue: package without source distribution gets an "incomplete" resolution (issue, open). Canonical tracking issue for universal resolution selecting wheel-only releases that cannot install on some platforms. Maintainers explicitly include PyQt5-Qt5 releases without Windows wheels and direct matching PyQt5 reports here.
- astral-sh/uv#13299 — uv add fails to install package while  uv pip install doesn't (issue, closed). Same Windows `uv add pyqt5` failure, involving PyQt5-Qt5 5.15.16 instead of 5.15.19. A maintainer recommended required-environments; the reporter confirmed it selected Windows-compatible PyQt5-Qt5 5.15.2 and installed successfully.
- astral-sh/uv#9907 — resolver: requiring `PyQt5` produces non-portable locks (issue, closed). Same dependency and missing Windows artifacts after `uv add PyQt5`. Maintainers explicitly closed it into astral-sh/uv#9711; it documents why Windows wheels for older releases do not make the selected newer release installable.
- astral-sh/uv#9928 — Fork when wheels are missing platform support (pull request, closed). Proposed general platform-based backtracking explicitly covered the PyQt5 case, but was closed without merging. Its discussion identifies the narrower astral-sh/uv#10046 as the change shipped instead.
- astral-sh/uv#10046 — Backtrack to non-local versions when wheels are missing platform support (pull request, merged). Merged December 20, 2024, but only handles local versions such as 2.5.1+cpu and their corresponding base versions. It does not fix selection among ordinary PyQt5-Qt5 releases, so this report does not establish a regression of that fix.

## Report decomposition

- Observable failure: `uv add PyQt5` resolves a dependency version that cannot be installed on the current Windows platform.
- Apparent contradiction: the reporter found a `PyQt5-5.15.11-cp38-abi3-win_amd64.whl` file, while the error identifies the separate `pyqt5-qt5==5.15.19` distribution.
- Expected capability: choose a dependency release that has usable Windows artifacts.
- Commands and subsystems: project dependency addition, universal resolution/locking, and installation-time wheel compatibility.
- Triggering conditions: Windows x64, Python 3.12, uv 0.13.0, and a wheel-only dependency release whose published artifacts cover Linux/macOS.
- Literal identifiers: `PyQt5`, `pyqt5-qt5`, `5.15.19`, `win_amd64`, `cp38-abi3`, `required-environments`, and “doesn't have a source distribution or wheel for the current platform”.

## Reported workaround and remaining reproduction details

The reporter reports a successful installation using a wheel manually downloaded from https://pypi.org/project/PyQt5/#files and initially interpreted the original failure as uv not knowing that wheel exists. The comment does not identify the downloaded filename or version, the installer or command used, whether the installation targeted the original project environment, or the installed `PyQt5-Qt5` version. It also does not confirm that `tool.uv.required-environments` was tried.

This outcome is compatible with the already established distinction between the PyQt5 bindings and their Qt runtime dependency. The original error names `PyQt5-Qt5==5.15.19`, so successful installation of a manually downloaded PyQt5 wheel does not by itself demonstrate a discovery or ABI-compatibility failure for the parent package. How the successful installation satisfied the runtime dependency remains unknown; neither an older dependency selection nor reuse of an installed dependency has been established for this attempt. The duplicate classification remains unchanged.

For a meaningful comparison with the failing `uv add PyQt5`, the missing evidence is the downloaded wheel filename, exact installation command and target environment, resulting PyQt5/PyQt5-Qt5 versions, and any project configuration or lockfile changes. These are investigation gaps, not new reproduction steps inferred from the comment.

## Supporting evidence

### Package metadata

Read-only public PyPI JSON metadata was checked for these exact releases:

- https://pypi.org/pypi/PyQt5/5.15.11/json includes `PyQt5-5.15.11-cp38-abi3-win_amd64.whl` and declares `PyQt5-Qt5>=5.15.2,<5.16.0`.
- https://pypi.org/pypi/PyQt5-Qt5/5.15.19/json lists exactly three wheels: macOS x86_64, macOS arm64, and manylinux2014 x86_64. It lists no Windows wheel or source distribution.
- https://pypi.org/pypi/PyQt5-Qt5/5.15.2/json includes `PyQt5_Qt5-5.15.2-py3-none-win_amd64.whl`. This is the older compatible dependency release confirmed in the historical workaround.

These facts establish why the cited parent-package wheel does not satisfy the dependency named in the error.

### Package roles and Python compatibility

The reporter's follow-up in astral-sh/uv#22716 asks what distinguishes `PyQt5` from `PyQt5-Qt5`. It adds no reproduction configuration or confirmation that the required-environments workaround was tried. The duplicate classification remains applicable.

Public PyPI metadata describes `PyQt5` as the Python bindings for the Qt application toolkit and `PyQt5-Qt5` as the subset of a Qt installation needed by PyQt5. The former supplies the Python bindings; the latter supplies the Qt runtime libraries they need. They are separate distributions with separate release files. The already verified `PyQt5==5.15.11` metadata declares a dependency on `PyQt5-Qt5>=5.15.2,<5.16.0`, so availability of the bindings' Windows wheel alone does not make the selected dependency release installable. These roles were checked against the `summary` fields at https://pypi.org/pypi/PyQt5/5.15.11/json and https://pypi.org/pypi/PyQt5-Qt5/5.15.19/json.

The participant has withdrawn the claim that PyQt5 Windows wheels are only for Python 3.8 and now recognizes their stable-ABI compatibility. The follow-up wording “any Python version” is too broad: for the cited `cp38-abi3-win_amd64` wheel, `cp38` with `abi3` denotes a CPython 3.8 stable-ABI baseline, compatible with later standard CPython versions, including 3.12, on the matching platform. PyQt5 5.15.11 declares `Requires-Python: >=3.8`. In this checkout, `crates/uv-platform-tags/src/tags.rs:242` generates compatible stable-ABI tags for earlier Python minor versions, including `cp38-abi3` for CPython 3.12. There is no evidence here of a Python 3.12 incompatibility in that parent-package wheel.

The discussion now distinguishes the missing Windows artifacts in `PyQt5-Qt5==5.15.19` from the Windows wheels available in 5.15.2, and suggests trying `tool.uv.required-environments`. These version-specific availability facts agree with the verified metadata above. The participant’s stronger assertion that 5.15.2 is the only release ever to provide Windows wheels has not been independently verified across all releases. The follow-up is not a maintainer decision and provides no reporter confirmation that the required-environments workaround was tried or succeeded.

### Repository documentation and maintainer findings

`docs/concepts/resolution.md:175` explains required environments. Lines 191–209 describe the default wheel-availability requirement and how `required-environments` makes resolution require platform coverage and backtrack past newer releases. `docs/concepts/projects/config.md:618` documents the same setting. Public documentation: https://docs.astral.sh/uv/concepts/resolution/#required-environments.

In astral-sh/uv#13299, maintainer konstin explains that universal resolution selects a newer PyQt5-Qt5 release while platform-specific installation can select an older compatible release. The reporter confirms that switching from `environments` to `required-environments` resolves the error and installs PyQt5-Qt5 5.15.2.

In astral-sh/uv#9711, the December 18, 2024 maintainer categorization explicitly includes PyQt5 releases missing Windows wheels. The December 20 follow-up says the shipped change is limited to local versus base versions. The March 20, 2025 follow-up recommends required environments.

### Fix history and boundaries

astral-sh/uv#7005 and astral-sh/uv#8603 contain earlier matching Windows errors and initially point to astral-sh/uv#5182. That older PyTorch issue was closed by astral-sh/uv#10046. The PR body explicitly limits the fix to local versions and their corresponding non-local alternatives. The broader astral-sh/uv#9928 explicitly covered PyQt5, but was closed without merging. The open astral-sh/uv#9711 remains the appropriate general discussion.

The inspected historical changes date from 2024, before the reported uv 0.13.0. They do not establish that automatic selection among ordinary PyQt5-Qt5 releases was fixed and subsequently regressed.

## Search scope and candidate distinctions

Searched astral-sh/uv open/closed issues and open/closed/merged PRs with authenticated gh. Literal searches covered PyQt5, pyqt5-qt5, 5.15.19, cp38-abi3-win_amd64, and the missing-source-distribution-or-wheel error. Conceptual searches covered current-platform version selection, required-environments, incomplete universal resolutions, and missing platform support; fix searches also checked closed 0.13.0 wheel reports. PR keyword searches returned no matches; discussion links led to the directly inspected historical PRs. Read candidate comments and followed astral-sh/uv#7005 and astral-sh/uv#8603 through astral-sh/uv#5182, plus astral-sh/uv#9907 to astral-sh/uv#9711. Ruled out astral-sh/uv#19308 as the canonical match because it alleges ignored explicit Windows pins and required-environments, neither reported here; astral-sh/uv#15421 concerns a source-build failure. Also reviewed astral-sh/uv#11865 and astral-sh/uv#11855, repository documentation, and public PyPI release metadata.

astral-sh/uv#11865 shares the Windows missing-wheel symptom, but originally also alleged an ignored exact pin and was closed after no minimal reproduction was provided. Later comments recommend required environments or explicit compatible dependency versions. It is supporting history rather than the clearest canonical discussion.

astral-sh/uv#19308 is especially plausible because it names the same dependency and platform, but it explicitly supplies Windows-specific pins and required environments, then alleges selection of the non-Windows lockfile entry. The new issue supplies no such configuration.

astral-sh/uv#15421 fails inside the SIP/qmake source-build backend for an older PyQt5 release, unlike this report's missing artifacts for PyQt5-Qt5.

astral-sh/uv#11855 requests requiring the current platform by default. Maintainers discuss keeping locking independent of the current machine. It is adjacent policy/design context; the new report can be centralized in the existing incomplete-resolution tracker.

## Verification and limits

The issue, candidate discussions, historical pull requests, repository documentation, and public package metadata were inspected read-only. No installation or Windows reproduction was run during this investigation, so the suggested configuration has not been tested against the reporter's undisclosed project constraints. The manual-wheel installation success is reported by the user and has not been independently verified. The setting and earlier successful PyQt5 use are supported by documentation and maintainer/reporter discussion.

No GitHub changes were made. No checkout files were modified by this triage. Only this issue-context README was authored for the handoff.
