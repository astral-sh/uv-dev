# 0.13.0: `uv build` rejects sdists with full-width owner/group fields

Issue: astral-sh/uv#22510

## Summary

**Reproduced.** On Linux x86_64 with the installed uv 0.13.0 and CPython 3.12.3, `uv build --wheel` rejects a CPython `tarfile` PAX-format sdist whose ASCII `gname` is 32 or 40 bytes long. The exact errors match the report: `invalid ustar gname field: missing NUL terminator`, at byte 0 and byte 1024 respectively. A 31-byte group name builds successfully. uv 0.12.24 builds both rejected archives, and uv 0.13.0 with `UV_LEGACY_TAR_BACKEND=1` builds the 40-byte case.

The analogous `uname` cases were also independently reproduced: 32 and 40 bytes fail with the corresponding `uname` error, while the 40-byte case builds with the legacy backend. This does not require a real long-named system group or an Azure container: setting the owner/group metadata on synthetic archive members is sufficient.

The reporter encountered this after upgrading to uv 0.13.0 in Azure Pipelines containers with long domain group names, and reported Windows 11 x86_64 and Linux x86_64 with Python 3.13.12. This reproduction verifies Linux with Python 3.12.3; Windows and Python 3.13.12 were not independently tested.

Maintainer `woodruffw` identifies astral-sh/uv#22522 as the intended fix. The independently reviewed PR is open and marked draft: it upgrades tar-codec to 0.0.15 and enables a compatibility option for owner/group headers without NUL terminators. Its test plan says it still needs a test; the fix has not been independently exercised, and no merge or release is confirmed. The specification requires NUL-terminated owner/group fields, so this is a compatibility allowance for CPython-generated archives rather than a change to that requirement.

## Classification

Bug: an observed compatibility regression at the tar-backend switch in uv 0.13.0. CPython writes these archive headers, Python and GNU tar read them, and uv's previous backend successfully builds wheels from the same inputs. The requested behavior is acceptance of full-width owner/group fields, including entries with PAX overrides.

The classification describes the reproduced compatibility regression. It does not imply that the CPython-generated headers conform to the archive specification: the owner/group NUL-termination requirement supports the maintainer's concern about Python's output. The reproduction establishes the behavior and regression boundary; the source changes in astral-sh/uv#22522 now identify the proposed compatibility mechanism. The draft PR has not been independently tested here, and dependency-internal validation and PAX override ordering remain uninspected.

Prior triage found no duplicate. The separate hard-link rejection in astral-sh/uv#22492 and historical PAX streaming bug concern different mechanisms. astral-sh/uv#22522 is a fix proposed in response to this issue, so its existence does not change the bug classification to duplicate.

## Format conformance and maintainer assessment

Maintainer `woodruffw` interprets the missing terminator as likely Python non-conformance and says it should be corrected on Python's side. For uv compatibility, the maintainer identifies astral-sh/uv#22522 as the fix in the October 13, 2026, 17:38 UTC comment on astral-sh/uv#22510. The PR implements the suggested limited allowance, but remains an open draft. The supplied discussion does not identify an upstream Python issue or a uv release target.

The cited [POSIX pax specification](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/pax.html) was independently checked. Its ustar header description requires `magic`, `uname`, and `gname` to be character strings terminated by NUL. The exception allowing all bytes to be non-NUL applies to `name`, `linkname`, and `prefix`, not to owner/group fields. The reproduced 32- and 40-byte fixtures have no NUL in the fixed owner/group field, so the observed bytes conflict with that requirement. This standards evidence qualifies the reporter's expected behavior while leaving the demonstrated compatibility regression intact. It does not independently settle how the decoder should handle a PAX override of the affected field.

## Reproduction

Outcome: **reproducible**.

### Environment and isolation

- Linux x86_64, kernel `6.17.0-1022-azure`.
- Installed executable on `PATH`: `/opt/hostedtoolcache/uv/0.13.0/x86_64/uv`; `uv --version` reports `uv 0.13.0 (x86_64-unknown-linux-gnu)`.
- CPython 3.12.3 at `/usr/bin/python3`, explicitly selected for builds. Automatic Python downloads disabled.
- Build backend: `setuptools.build_meta`, declared requirement `setuptools>=77`; successful builds resolved setuptools 84.0.0.
- Comparison executable: uv 0.12.24, obtained using the installed uv's `tool run --from uv==0.12.24 uv`; its version output was checked.
- GNU tar 1.35 was used for an independent archive-reading check.
- All scripts, archives, wheels, logs, caches, tool environments, and temporary build directories are under `/tmp/uv-22510-BtAUQv`. The launcher uses a fresh environment, no user configuration, disabled keyring integration, and temporary cache/config/data/credential-storage paths. No checkout files or existing user state were changed.

### Minimal fixture and commands

The independently reconstructed fixture contains only `pyproject.toml`, an empty Python package, and `PKG-INFO`. Actual TOML/metadata newlines are written. Save this as `make_sdist.py` in a new temporary directory:

```python
import io
from pathlib import Path
import sys
import tarfile

length = int(sys.argv[1])
field = sys.argv[2] if len(sys.argv) > 2 else "gname"
assert field in {"gname", "uname"}
output = Path(f"{field}-{length}")
output.mkdir(exist_ok=True)
files = {
    "demo-0.1.0/pyproject.toml": (
        b'[project]\nname = "demo"\nversion = "0.1.0"\n'
        b'[build-system]\nrequires = ["setuptools>=77"]\n'
        b'build-backend = "setuptools.build_meta"\n'
    ),
    "demo-0.1.0/demo/__init__.py": b"",
    "demo-0.1.0/PKG-INFO": (
        b"Metadata-Version: 2.1\nName: demo\nVersion: 0.1.0\n"
    ),
}
with tarfile.open(
    output / "demo-0.1.0.tar.gz", "w:gz", format=tarfile.PAX_FORMAT
) as archive:
    for name, data in files.items():
        member = tarfile.TarInfo(name)
        member.size = len(data)
        setattr(member, field, "g" * length)
        archive.addfile(member, io.BytesIO(data))
```

With uv 0.13.0 on `PATH`, these are the core invocations performed by the isolated launcher. Run from the temporary directory, with temporary state paths and an explicit available Python interpreter:

```sh
export TMPDIR="$PWD/tmp" UV_CACHE_DIR="$PWD/cache"
export UV_TOOL_DIR="$PWD/tools" UV_TOOL_BIN_DIR="$PWD/tool-bin"
export UV_PYTHON_INSTALL_DIR="$PWD/python" UV_CREDENTIALS_DIR="$PWD/credentials"
export XDG_CONFIG_HOME="$PWD/xdg-config" XDG_DATA_HOME="$PWD/xdg-data"
export XDG_CACHE_HOME="$PWD/xdg-cache"
export UV_NO_CONFIG=1 UV_PYTHON=/usr/bin/python3 UV_PYTHON_DOWNLOADS=never
export UV_KEYRING_PROVIDER=disabled PYTHONNOUSERSITE=1
unset UV_LEGACY_TAR_BACKEND
mkdir -p "$TMPDIR"

python3 make_sdist.py 31
python3 make_sdist.py 32
python3 make_sdist.py 40

uv build --wheel gname-31/demo-0.1.0.tar.gz --out-dir out/default-31
uv build --wheel gname-32/demo-0.1.0.tar.gz --out-dir out/default-32
uv build --wheel gname-40/demo-0.1.0.tar.gz --out-dir out/default-40
UV_LEGACY_TAR_BACKEND=1 uv build --wheel gname-40/demo-0.1.0.tar.gz --out-dir out/legacy-40 --no-build-logs

uv tool run --from uv==0.12.24 uv --version
uv tool run --from uv==0.12.24 uv build --wheel gname-32/demo-0.1.0.tar.gz --out-dir out/old-32 --no-build-logs
uv tool run --from uv==0.12.24 uv build --wheel gname-40/demo-0.1.0.tar.gz --out-dir out/old-40 --no-build-logs

python3 make_sdist.py 32 uname
python3 make_sdist.py 40 uname
uv build --wheel uname-32/demo-0.1.0.tar.gz --out-dir out/default-uname-32
uv build --wheel uname-40/demo-0.1.0.tar.gz --out-dir out/default-uname-40
UV_LEGACY_TAR_BACKEND=1 uv build --wheel uname-40/demo-0.1.0.tar.gz --out-dir out/legacy-uname-40 --no-build-logs
```

### Observed results

Each success produced `demo-0.1.0-py3-none-any.whl`; each wheel was checked as a valid ZIP containing `demo/__init__.py`.

| Field and ASCII length | uv 0.13.0 default | uv 0.13.0 legacy | uv 0.12.24 default |
| --- | --- | --- | --- |
| `gname`, 31 | Exit 0, wheel built | Not run | Not run |
| `gname`, 32 | Exit 2, missing NUL at byte 0 | Not run | Exit 0, wheel built |
| `gname`, 40 | Exit 2, missing NUL at byte 1024 | Exit 0, wheel built | Exit 0, wheel built |
| `uname`, 32 | Exit 2, missing NUL at byte 0 | Not run | Not run |
| `uname`, 40 | Exit 2, missing NUL at byte 1024 | Exit 0, wheel built | Not run |

The 40-byte group-name failure was:

```text
error: Failed to build `/tmp/uv-22510-BtAUQv/gname-40/demo-0.1.0.tar.gz`
  cause: Invalid tar file
  cause: at byte 1024: invalid ustar gname field: missing NUL terminator
  cause: invalid ustar gname field: missing NUL terminator
```

The 32-byte case has the same error at byte 0. Both fail before the `Building wheel from source distribution...` message. Owner-name cases substitute `uname` for `gname`.

Independent inspection of the decompressed 512-byte headers confirmed:

- At 31 bytes, the `gname` field contains 31 ASCII `g` bytes followed by NUL, and there is no PAX group override.
- At 32 bytes, all 32 bytes of the field are `g`, without NUL or a PAX group override.
- At 40 bytes, the ordinary file header still contains 32 `g` bytes without NUL, and a preceding PAX record supplies all 40 characters. The first ordinary file header begins at byte 1024.
- The 32- and 40-byte `uname` cases have the same encoding pattern in the owner field.

Python `tarfile` read back the complete intended owner/group names for every generated fixture. `tar -tzf` successfully listed all three files in the 32- and 40-byte group-name archives. These observations support compatibility with common archive producers/readers without assuming that source inspection alone proves reproducibility.

Evidence is retained in `/tmp/uv-22510-BtAUQv`: `make_sdist.py`, `run_uv.py`, the named input directories, per-invocation `.log` and `.json` files recording output/arguments/exit codes, and `out/` wheel artifacts. No Rust build or repository test run was needed to observe the installed executable's behavior.

### Existing test coverage

Searched the relevant integration tests under `crates/uv/tests/`, `crates/uv-client/tests/it/`, and extraction implementation for owner/group fields, missing-NUL errors, PAX, and tar fixtures. No regression test for full-width `uname`/`gname` decoding was found. The following nearby tests were read, including setup and assertions:

- `crates/uv/tests/build/build.rs`, `build_wheel_from_sdist`: builds a Hatchling sdist, checks explicit `--wheel` requirements, and successfully builds a wheel even with `--no-build`. It does not set archive owner/group fields.
- `crates/uv/tests/build/build_backend.rs`, `build_sdist_with_long_path`: creates a 133-byte member path and a 5 MiB + 1-byte file; checks GNU-format output from the legacy writer and PAX path records, modes, and streamed contents from the default writer. Python reads the generated archives. It does not test decoding a full-width owner/group field.
- `crates/uv/tests/build/build.rs`, `venv_included_in_sdist`: checks rejection of an absolute virtual-environment symlink and the associated hint with both tar backends. This is a separate extraction-policy case.

## Related

- astral-sh/uv#22522 — Bump tar-codec to 0.0.15 (open draft; not merged at review). The maintainer explicitly identifies this as the fix for astral-sh/uv#22510. Its description and diff enable `allow_header_uname_and_gname_without_terminator(true)` in the extraction decode policy. No test files are changed, and the PR test plan says it needs a test.
- astral-sh/uv#22094 — Use tar-codec by default (merged October 1, 2026; included in uv 0.13.0). Establishes the extraction/backend switch and `UV_LEGACY_TAR_BACKEND=1` fallback; does not fix this field rejection. Its PR description was independently reviewed during reproduction.
- astral-sh/uv#22492 — v0.13.0 fails to install pybabel-json==0.2.0. Prior triage identified this as an open issue with maintainers recommending the legacy backend. Its error is `extraction policy rejected input: hard-link members are not allowed`, a distinct validation failure.
- astral-sh/uv#22143 — Share archive decoding across reader types (merged October 8, 2026). The independently reviewed PR description concerns sharing decoding through a borrowed reader; it does not implement an owner/group-field fix.

## Implementation and release evidence

- `CHANGELOG.md:66` documents the tar-codec default and fallback in uv 0.13.0, released October 9, 2026.
- `crates/uv-static/src/lib.rs:12` defines `TarCodec` as the default backend, and `TarBackend::from_env` selects `TokioTar` for the legacy setting.
- `crates/uv-extract/src/stream.rs:729` constructs the tar-codec reader using its default decoding policy plus specific PAX vendor-extension allowances. `untar_in` at line 876 dispatches to the selected backend.
- `Cargo.toml` and `Cargo.lock` identify tar-codec and tar-framing 0.0.14. Their dependency-internal owner-field validation and PAX application order were not inspected in this reproduction.

The backend comparison confirms that the same sdist succeeds when the extraction backend changes. The reviewed diff of astral-sh/uv#22522 supplies a concrete proposed fix: `Cargo.toml` upgrades tar-codec from 0.0.14 to 0.0.15, the lockfile also upgrades tar-framing and archive-trait to 0.0.15, and `untar_in_tar_codec` enables `DecodePolicy::allow_header_uname_and_gname_without_terminator(true)`. The code comment explicitly cites CPython's 32-byte owner/group encoding. Existing PAX vendor-extension allowances remain in place.

The PR description, changed-file list, and diff were read independently. Only `Cargo.toml`, `Cargo.lock`, and `crates/uv-extract/src/stream.rs` are changed; no regression test is included in the reviewed revision. The PR is marked draft, has no merge timestamp, and its test plan states that it needs a test. No patched executable was built or tested during this context update; the reproduction results above describe the original affected versions.

## Prior related-work search

The previous triage searched open/closed issues and PRs for the exact error, owner/group names, NUL termination, PAX, tarfile, and backend changes, and reviewed recent PR listings where keyword search missed known changes. No matching existing report or verified fix was found at that time. The maintainer has since identified the open draft fix astral-sh/uv#22522, described above.

Excluded candidates remain distinct: astral-sh/uv#5450 and its fix astral-sh/tokio-tar#40 (with historical context in astral-sh/tokio-tar#1) concern retaining PAX data across pending reads, fixed in uv 0.5.30; astral-sh/uv#12762 concerns writing long paths; astral-sh/uv#21641 concerns connection resets; astral-sh/uv#19883 and astral-sh/uv#16911 concern format support/conformance; astral-sh/uv#11223 concerns older hard-link path resolution. The closed, unmerged documentation proposal astral-sh/uv#22425 does not provide a fix.

## Recommended next step

Review and validate astral-sh/uv#22522, especially the missing regression coverage. Use the existing 31-byte control, 32-byte unterminated fields, and 40-byte PAX cases for both `uname` and `gname` to verify the proposed decode-policy allowance. Track merge and release status before describing the fix as available to users. Any CPython writer correction remains a separate upstream follow-up without an identified issue in the supplied discussion. The legacy-backend setting remains a verified workaround for the tested 40-byte group and owner names. The reporter also suggests building a wheel directly from the source tree with `uv build --wheel`; that separate workaround was not independently tested here.
