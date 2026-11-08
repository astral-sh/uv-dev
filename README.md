# `uv pip install ./file` fails when filename contains `#`

Issue: astral-sh/uv#23118

## Summary

Both reported failures were reproduced using a minimal, dependency-free source archive with package metadata `Name: aprict`, `Version: 0.0.38`:

1. `uv pip install './aprict-0.0.38-cupy-issue-#10458.tar.gz'` exits 1 with `Distribution not found`, although the archive exists.
2. Installing the same bytes as `aprict-0.0.38-cupy-issue-10458.tar.gz` exits 1 with `Package metadata name aprict does not match given name aprict-0-0-38-cupy-issue`.

Pip successfully installs both filenames. uv successfully installs the same bytes with the standard filename `aprict-0.0.38.tar.gz`, and also installs the original filename through an explicitly named, percent-encoded file URL.

The report uses uv 0.12.22 (70fe1196a, 2026-10-01), Python 3.14.0, and Arch Linux 7.2.8-arch1-2. The observed reproduction uses the installed uv 0.13.6 and Python 3.14.10 on Linux x86_64. The exact reported versions and original archive were not tested.

## Classification

Bug: reproduced local-path lookup failure for an existing source archive whose filename contains a literal `#`, with a verified difference from pip. The metadata-name mismatch after removing `#` is a separate reproduced behavior involving a nonstandard archive filename. Reproduction confirms the observations; it does not establish that arbitrary archive names conform to source-distribution filename standards or settle the desired policy for such names.

There is no confirmed regression point. This is not an established duplicate of astral-sh/uv#4917: that report and its fix concern GitHub archive URLs rather than local archives. Likewise, astral-sh/uv#2501 concerns an intentional URL fragment rather than a literal filename character. No canonical issue or pull request covering this precise case was identified in the prior related-issue investigation.

## Filename provenance and maintainer clarification

The reporter confirms in the discussion of astral-sh/uv#23118 that they manually renamed the files using `mv`. The reported filenames therefore result from a manual rename; they are not evidence that a build tool emitted those names. The original filenames and archive build steps have not been provided.

Maintainer woodruffw notes that PEP 625 source-distribution filenames must follow `{name}-{version}.tar.gz`, with neither component containing `#`. The reporter's clarification narrows the case to installing manually renamed archives with nonstandard filenames. It does not establish whether the original archives were conforming sdists or general source archives. Woodruffw favors not supporting this case as part of stricter PEP 625 conformance and explicitly connects it to astral-sh/uv#19883. This is a tentative maintainer preference, not a final decision or evidence that the observed diagnostics are appropriate.

Maintainer charliermarsh explicitly leaves support for this case undecided and identifies a parsing difference: uv treats the suffix as a fragment, while pip appears to treat a bare path literally and require a `file://` URL to attach SHA hash fragments. The uv interpretation agrees with the source findings below, and pip's literal-filename behavior agrees with the existing reproduction. The statement about pip requiring a file URL for SHA fragments is a maintainer observation; that distinction was not independently tested in the recorded reproduction.

The existing reproduction, bug classification, and verified workaround remain unchanged. The synthetic fixture demonstrates the effect of different filenames on identical archive bytes, consistent with the reported manual rename. The open compatibility decision is whether bare local paths should treat `#` literally, how that would interact with uv's existing support for intentional path fragments described in astral-sh/uv#2501 and astral-sh/uv#2502, and whether diagnostics should improve if these names remain unsupported. No final support decision or commitment to a fix appears in the supplied discussion. The original archive and build details remain unavailable for validation against the reporter's package.

## Reproduction

**Outcome: reproducible.** The two reported errors and the pip comparison were observed directly.

### Environment and isolation

- Executable from `PATH`: `/opt/hostedtoolcache/uv/0.13.6/x86_64/uv`; version output: `uv 0.13.6 (x86_64-unknown-linux-gnu)`.
- Interpreter: `/opt/hostedtoolcache/Python/3.14.10/x64/bin/python3`; CPython 3.14.10.
- Platform: Linux 7.0.0-1017-azure, x86_64.
- Comparison: pip 26.2.2 running with the same interpreter.
- All files, virtual environments, home/config directories, temporary build files, and caches were isolated under `/tmp/uv-23118-6yd8bah6`. The test working directory was `cupy-issue-#10458`, matching the report's literal `#` in the parent directory.
- Commands ran with a fresh environment, `UV_NO_CONFIG=1`, `UV_OFFLINE=1`, `UV_PYTHON_DOWNLOADS=never`, and temporary `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR`, `HOME`, `TMPDIR`, and XDG directories. No package index or external build dependencies were needed. pip used `--isolated --no-index --no-cache-dir` and `PIP_CONFIG_FILE=/dev/null`.

### Minimal fixture

The archive contains one top-level `aprict-0.0.38` directory with the following two files. Its local PEP 517 backend generates a pure Python wheel with matching metadata and a module whose `__version__` is `0.0.38`. The archive has no runtime or build dependencies.

`pyproject.toml`:

```toml
[build-system]
requires = []
build-backend = "backend"
backend-path = ["."]

[project]
name = "aprict"
version = "0.0.38"
```

`backend.py`:

```python
from pathlib import Path
import base64
import csv
import hashlib
import io
import zipfile

INFO = "aprict-0.0.38.dist-info"
METADATA = "Metadata-Version: 2.2\nName: aprict\nVersion: 0.0.38\n\n"
WHEEL = "Wheel-Version: 1.0\nGenerator: local-reproduction\nRoot-Is-Purelib: true\nTag: py3-none-any\n\n"

def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    directory = Path(metadata_directory) / INFO
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "METADATA").write_text(METADATA)
    (directory / "WHEEL").write_text(WHEEL)
    return INFO

def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    filename = "aprict-0.0.38-py3-none-any.whl"
    files = {
        "aprict.py": b"__version__ = '0.0.38'\n",
        INFO + "/METADATA": METADATA.encode(),
        INFO + "/WHEEL": WHEEL.encode(),
    }
    records = io.StringIO()
    writer = csv.writer(records, lineterminator="\n")
    for name, data in files.items():
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
        writer.writerow([name, "sha256=" + digest, len(data)])
    writer.writerow([INFO + "/RECORD", "", ""])
    files[INFO + "/RECORD"] = records.getvalue().encode()
    with zipfile.ZipFile(Path(wheel_directory) / filename, "w") as wheel:
        for name, data in files.items():
            wheel.writestr(name, data)
    return filename
```

From a fresh temporary directory containing that source directory, create three byte-identical archives:

```sh
mkdir 'cupy-issue-#10458'
python3 - <<'PYTHON'
import tarfile
with tarfile.open('cupy-issue-#10458/aprict-0.0.38-cupy-issue-#10458.tar.gz', 'w:gz') as archive:
    archive.add('aprict-0.0.38', arcname='aprict-0.0.38')
PYTHON
cd 'cupy-issue-#10458'
cp 'aprict-0.0.38-cupy-issue-#10458.tar.gz' aprict-0.0.38-cupy-issue-10458.tar.gz
cp 'aprict-0.0.38-cupy-issue-#10458.tar.gz' aprict-0.0.38.tar.gz
uv venv --python /opt/hostedtoolcache/Python/3.14.10/x64/bin/python3 .venv
```

Use the isolation settings above when running these commands. The fixture and archives already exist under `/tmp/uv-23118-6yd8bah6` for this run.

### Observed failures

```sh
uv pip install './aprict-0.0.38-cupy-issue-#10458.tar.gz'
```

Exit status 1:

```text
error: Distribution not found at: file:///tmp/uv-23118-6yd8bah6/cupy-issue-%2310458/aprict-0.0.38-cupy-issue-#10458.tar.gz
```

```sh
uv pip install ./aprict-0.0.38-cupy-issue-10458.tar.gz
```

Exit status 1:

```text
error: Failed to build `aprict-0-0-38-cupy-issue @ file:///tmp/uv-23118-6yd8bah6/cupy-issue-%2310458/aprict-0.0.38-cupy-issue-10458.tar.gz`
  cause: Package metadata name `aprict` does not match given name `aprict-0-0-38-cupy-issue`
```

The expected behavior reported by the user is successful installation, as with pip.

### Controls and pip comparison

```sh
uv pip install ./aprict-0.0.38.tar.gz
.venv/bin/python -c 'import aprict; print(aprict.__version__)'
```

Both exit 0; uv builds and installs `aprict==0.0.38`, and the import prints `0.0.38`. This also confirms that the literal `#` in the working directory does not by itself prevent installation of the relative archive path.

A separate virtual environment was used for pip:

```sh
uv venv --python /opt/hostedtoolcache/Python/3.14.10/x64/bin/python3 pip-venv
/opt/hostedtoolcache/Python/3.14.10/x64/bin/python3 -m pip --isolated --python "$PWD/pip-venv/bin/python" install --no-index --no-cache-dir './aprict-0.0.38-cupy-issue-#10458.tar.gz'
/opt/hostedtoolcache/Python/3.14.10/x64/bin/python3 -m pip --isolated --python "$PWD/pip-venv/bin/python" install --no-index --no-cache-dir --force-reinstall ./aprict-0.0.38-cupy-issue-10458.tar.gz
pip-venv/bin/python -c 'import aprict; print(aprict.__version__)'
```

Both pip installations exit 0 with `Successfully installed aprict-0.0.38`; the final import prints `0.0.38`.

### Existing test coverage

Searched the integration tests under `crates/uv/tests/` (including `it/` and the current `pip_install/` and `pip/` modules), `crates/uv-client/tests/it/`, and the relevant parser tests. No integration test for a literal `#` in a local source-archive filename was identified in this search. Related tests were read through their setup and assertions:

- `crates/uv-pep508/src/verbatim_url.rs`, `hash_in_path`: checks literal-`#` encoding and file-path round trips through `from_absolute_path`, `from_normalized_path`, and `from_path`. It does not exercise `from_path_with_fragment` or `uv pip install`.
- `crates/uv-pep508/src/verbatim_url.rs`, `fragment` (gated by `non-pep508-extensions`): expects `#hash=somehash` to be separated from supplied paths, covering intentional fragment syntax.
- `crates/uv/tests/lock/lock.rs`, `lock_workspace_member_with_fragment_delimiter` (gated by `test-universal`): locks a workspace member directory named `member#x` and snapshots the editable member path. It does not install an archive supplied on the command line.
- `crates/uv/tests/pip_install/direct_url_hashes.rs`, `require_hashes_accepts_direct_url_hash_from_explicit_requirement`: installs a local source archive through a named file URL with a `#sha256=...` fragment, checking successful installation and backend execution. The fragment is intentional; the archive filename has no literal `#`.

These are adjacent behaviors, not existing coverage of the reported failure. No checkout tests were added or run; the checks above used the installed executable directly.

### Evidence and limits

The original archive was not supplied. The synthetic fixture reproduces both exact classes of error and the successful pip comparison without the original project's implementation or dependencies. No comparison with uv 0.12.22 or Python 3.14.0 was performed, and no version introducing the behavior is established.

Raw command arguments, exit codes, stdout, and stderr are retained in `/tmp/uv-23118-6yd8bah6/results.json`. The fixture source and all three archives remain under the same temporary directory.

## Verified workaround

An explicitly named requirement with an absolute file URL and `%23` for every literal `#` installed successfully into another fresh environment:

```sh
uv venv --python /opt/hostedtoolcache/Python/3.14.10/x64/bin/python3 workaround-venv
uv pip install --python "$PWD/workaround-venv/bin/python" 'aprict @ file:///tmp/uv-23118-6yd8bah6/cupy-issue-%2310458/aprict-0.0.38-cupy-issue-%2310458.tar.gz'
workaround-venv/bin/python -c 'import aprict; print(aprict.__version__)'
```

All commands exit 0; installation reports `aprict==0.0.38` and the import prints `0.0.38`. This verifies the workaround for the minimal fixture, not the reporter's unavailable archive. Replacing the original filename with the standard `aprict-0.0.38.tar.gz` also works for the fixture.

## Implementation context

The observed errors are consistent with the following source paths; these observations do not replace execution evidence:

- `crates/uv-pep508/src/unnamed.rs`: local unnamed requirements delegate to `VerbatimUrl::from_path_with_fragment`.
- `crates/uv-pep508/src/verbatim_url.rs`: `from_path_with_fragment` splits the supplied path with `split_fragment`, which uses the first `#`, then restores the suffix as a URL fragment. The base directory is resolved separately, consistent with its encoded `#` in the observed error.
- `crates/uv-requirements/src/unnamed.rs`: source archive filenames are used to infer names, except for GitHub archive URLs.
- `crates/uv-distribution-filename/src/source_dist.rs`: `parsed_normalized_filename` removes the extension and splits at the final hyphen. Applied to the renamed filename, this produces the normalized name `aprict-0-0-38-cupy-issue` and version `10458`, consistent with the observed metadata mismatch.

## Related

- astral-sh/uv#4917 (issue, closed): `uv pip install https://...zip` fails where `pip install` works. The same metadata-name mismatch occurred when a GitHub archive filename was mistaken for a distribution name. A maintainer confirmed the bug; astral-sh/uv#4928 fixed GitHub archive URLs specifically. The new report concerns a local archive and additionally a literal # in its filename, so this is historical context rather than an established regression.
- astral-sh/uv#4928 (pull request, merged): Avoid inferring package name for GitHub Archives. Merged on 2024-07-09 to fix astral-sh/uv#4917. Its diff skips filename-based name inference only for github.com URLs containing an archive path segment. That exception remains in the checkout and does not cover the reported local .tar.gz file.
- astral-sh/uv#2501 (issue, closed): uv fails to install wheel from path with fragment. Also produced Distribution not found for a local path containing #, but the reporter intended #hash=somehash as a URL fragment. Here #10458 is part of the actual archive filename: the intended interpretation is the opposite.
- astral-sh/uv#2502 (pull request, merged): Allow direct file path requirements to include fragments. Merged on 2024-03-18 to fix astral-sh/uv#2501 by separating the path at # and restoring the suffix as a URL fragment. The checkout still splits supplied paths this way, explaining the relevant parsing behavior; the PR did not add support for literal # characters in filenames.

- astral-sh/uv#19883 (issue): Roadmap: strict PEP 625 conformance. Woodruffw explicitly cites this roadmap when favoring rejection of the manually renamed, nonstandard archive filenames. It is relevant policy context for deciding support, rather than a confirmed fix or duplicate discussion for the literal-`#` lookup failure.

## Prior related-issue search

The earlier context investigation searched open and closed issues and pull requests in astral-sh/uv for literal-`#` path handling, fragments, percent encoding, local/source archive names, and the two reported diagnostic messages. It inspected the historical fixes listed above and found no canonical discussion or fix for this precise local-filename case. That GitHub search was retained as context and was not repeated during behavioral reproduction.

Previously excluded candidates remain distinct: astral-sh/uv#7714 concerns stale `PKG-INFO`; astral-sh/uv#9743 and astral-sh/uv#2727 concern Git/subdirectory metadata selection; astral-sh/uv#14345 concerns commas inside wheel `RECORD`; and astral-sh/uv#17526 concerns HTTP `Content-Disposition`. None establishes prior coverage of literal `#` in the installation argument. Although astral-sh/uv#19883 was previously excluded as a canonical match for the failure, it is now explicitly linked by a maintainer as relevant conformance policy and is included under Related on that basis.

## Changes made

Only this issue-context README and temporary reproduction files were written. No repository checkout files or existing Python environments were modified by this work, and no GitHub comments, labels, or other GitHub changes were made.
