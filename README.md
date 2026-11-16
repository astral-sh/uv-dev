# Not able to create venv for pyodide with python 3.15

Issue: astral-sh/uv#23262

Classification: enhancement

## Summary

The reporter cannot create a Pyodide virtual environment for Python 3.15 and notes that preview builds are already available. The live issue additionally links to https://pyodide.org/en/stable/usage/cli.html. It supplies no command, error output, uv version, operating system, exact Pyodide release, or indication of whether the runtime is managed by uv or installed separately.

uv supports Pyodide, but its managed-download metadata generator deliberately excludes Pyodide prereleases. The checked-in catalog contains five Emscripten distributions, with Python versions from 3.11.3 through 3.14.2, and none for 3.15. Upstream releases 315.0.0a1 and 315.0.0a2 exist and are marked as prereleases. This establishes a managed-download limitation; it does not establish the exact failure encountered by the reporter.

astral-sh/uv#18958 documents the deliberate exclusion of Pyodide prereleases. astral-sh/uv#16023 tracks separate listing and installation-name problems. No existing tracker specifically requesting Python 3.15 Pyodide previews was found.

## Draft response

uv currently skips Pyodide prereleases when generating its managed-Python catalog, as discussed in astral-sh/uv#18958. That excludes the available 315.0.0a1/a2 builds; supporting these previews would require extending managed-download support. Please share your exact command, uv version, platform, and full error output, and clarify whether uv is downloading Pyodide or using a runtime you installed separately.

## Classification

The supported-download generator deliberately skips Pyodide prereleases, and the checked-in Emscripten catalog has Python versions through 3.14.2 but no 3.15 entry. Making the available Pyodide 315 previews installable through managed downloads extends that capability. No command or error establishes a separate correctness bug with an already installed runtime, and no same-request tracker or previously fixed regression was identified.

The review of astral-sh/uv#18958 discusses prerelease build tags as an unsupported concept and recommends ignoring Pyodide prereleases. This is evidence for a request to extend managed-download support, rather than evidence of a broken supported download or a regression. The existing open installation-name issue has a different demonstrated trigger, and the report does not provide enough detail to equate them. Reclassify if command/error details establish a separate failure using an installed Python 3.15 runtime.

## Related

- astral-sh/uv#18958 — Sync latest Python releases (merged pull request). The review explicitly discusses excluding prerelease Pyodide builds because prerelease build tags are unsupported. The current Pyodide metadata generator cites this discussion and skips prereleases, directly explaining why the 315 preview builds are absent from managed downloads. This is an intentional limitation, not a previous fix for Python 3.15.
- astral-sh/uv#16023 — A few issues with pyodide + uv python (open issue). Tracks Pyodide versions being hidden without --all-platforms and installation failing with a listed pyodide-prefixed identifier while its cpython-prefixed equivalent works; a maintainer confirms the latter is a bug. These could affect interpreter selection, but concern existing releases rather than missing Python 3.15 previews, so the sparse report does not establish a duplicate.

## Supporting evidence

- `docs/concepts/python-versions.md:431` documents Pyodide implementation support; the managed-distribution section at line 475 identifies the Pyodide project as its distribution provider.
- `crates/uv-python-managed/fetch-download-metadata.py:520` defines `PyodideFinder`. Its release loop at lines 557–562 explicitly skips releases with `prerelease: true` and cites the review discussion in astral-sh/uv#18958.
- The review thread at https://github.com/astral-sh/uv/pull/18958#discussion_r3082735525 recommends ignoring Pyodide prereleases. The preceding maintainer comment says prerelease build tags are not supported. That pull request merged on April 16, 2026, before this November 16 report.
- `crates/uv-python-managed/download-metadata.json` includes `cpython-3.14.2-emscripten-wasm32-musl`, using Pyodide 314.0.7, and older Emscripten entries. There is no Python 3.15 Emscripten entry.
- The upstream GitHub releases API reports 315.0.0a1 and 315.0.0a2, published August 1 and August 4, 2026, with `prerelease: true`. Both contain corresponding `xbuildenv-315.0.0a*.tar.gz` assets. Availability upstream therefore does not imply inclusion in uv's catalog.
- astral-sh/uv#16023 has a maintainer-confirmed mismatch between listed `pyodide-...` names and installable `cpython-...` identifiers. The reporter here gives no identifier, so that mechanism remains unconfirmed.

## Other candidates inspected

astral-sh/uv#21199 is an open Windows path-layout bug during package installation into a successfully created Python 3.13 Pyodide environment. It does not establish a Python 3.15 creation failure. astral-sh/uv#15260 concerns missing Node at runtime after installation; no such error is reported here.

astral-sh/uv#19911 concerns native CPython 3.15 wheel resolution from a custom index; Pyodide appears incidentally in its logs. astral-sh/uv#22439 and astral-sh/uv#17240 concern native CPython distribution metadata becoming available with a uv release, rather than intentional Pyodide prerelease exclusion.

astral-sh/uv#12729 is the closed original Pyodide-support request; its comments lead to merged astral-sh/uv#12731, which added initial pip support. Neither tracks adding current Pyodide preview downloads. astral-sh/uv#19897 and its referenced merged refactor astral-sh/uv#19891 concern implementation short names.

## Search scope

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests using Pyodide, Python 3.15, 315, venv, virtual environment, Emscripten, PyEmscripten, preview, prerelease, xbuildenv, and download-availability terminology. No error or exact command was supplied. Separated environment-creation and interpreter-selection symptoms from catalog/prerelease-policy searches, and inspected candidate comments and referenced pull requests. Search endpoints returned no matching pull requests; referenced pull requests were inspected directly, including astral-sh/uv#18958 and its review thread. Ruled out astral-sh/uv#21199 (Windows package installation after successful venv creation), astral-sh/uv#19911 (wheel resolution), and astral-sh/uv#22439 and astral-sh/uv#17240 (CPython catalog release timing). Historical astral-sh/uv#12729 and astral-sh/uv#12731 concern initial Pyodide support, not preview downloads. Checked current source/catalog and upstream 315 prerelease assets.

## Next step

Request the command, full error, uv version, platform, and runtime installation method. For a managed download, the concrete capability to discuss is opting into Pyodide prerelease builds. A failure using a separately installed runtime needs its own diagnosis; no working workaround for that case was verified.

The checkout and GitHub were not modified. No builds or integration scenarios were run; the handoff relies on source inspection, issue/review evidence, catalog inspection, and upstream release metadata.
