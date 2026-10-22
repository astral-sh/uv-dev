# 0.13.5 missing from the astral-sh/versions manifest: publish-versions failed ("No interpreter found for Python 3.14.9")

Issue: astral-sh/uv#22826

Classification: bug

## Summary

Reproduced on Linux x86_64 with the installed uv 0.13.4. A minimal PEP 723 script requiring Python >=3.12, run beside a `.python-versions` file containing `3.14.9`, exits with code 2 and the reported interpreter error. The same script succeeds with the previous `3.14.8` pin. Keeping the `3.14.9` pin and supplying the checkout's Python download metadata also succeeds with uv 0.13.4.

The live `astral-sh/versions` manifest was checked on 2026-10-22: it contains 327 version records, with 0.13.4 newest and no 0.13.5 record. The 0.13.5 GitHub release is published. This leaves setup-uv's default manifest lookup without a 0.13.5 artifact on any platform; installations already satisfied by a tool cache need not perform that lookup.

The release's manifest and documentation publishing jobs pin uv 0.13.4 but do not configure the checkout's Python download metadata. The closest prior fix, astral-sh/uv#22810, configured other release stages and omitted these reusable workflows. astral-sh/uv#22802 introduced the Python 3.14.9 pin. astral-sh/uv#22793 concerns a separate earlier cancellation.

## Classification

This is a release-publication bug. The local interpreter failure is expected when an exact compatible Python pin is unavailable locally and absent from the running uv's embedded download catalog. The incorrect outcome is that the release's publishing workflows use that combination and leave the already-published uv version absent from the installation manifest.

The reproduction confirms the interpreter/catalog mismatch and verifies that the checked-in catalog resolves it. It does not indicate a regression in uv 0.13.5's interpreter discovery: the failing executable is uv 0.13.4. No additional reporter configuration is needed to reproduce this failure.

## Reproduction

**Outcome: reproducible.** Checked on 2026-10-22, approximately 06:55 UTC.

### Environment and fixture

- Linux x86_64; installed executable `/opt/hostedtoolcache/uv/0.13.4/x86_64/uv` reports `uv 0.13.4 (x86_64-unknown-linux-gnu)`.
- System `python3` reports 3.12.3. Each scenario uses a fresh managed-Python installation directory, so neither requested patch version is supplied by an existing managed installation.
- All fixtures, caches, downloaded interpreters, and evidence are under `/tmp/uv-22826-obx7u66v`. Commands use a clean environment with temporary home/config/data locations. No existing user configuration or credentials are passed to uv.
- The publishing scripts in `astral-sh/versions` declare PEP 723 `requires-python = ">=3.12"`. The release checkout's first `.python-versions` entry is `3.14.9`. Both publishing workflows set `UV_LOCKED=1`; neither sets `UV_PYTHON_DOWNLOADS_JSON_URL`.
- The minimal script retains the relevant PEP 723 requirement and replaces publication code with printing the interpreter version. It needs no project, dependency groups, third-party packages, release-plan input, or publication permissions to reach the reported failure.

Equivalent minimal commands, using the installed uv on PATH:

```bash
repro="$(mktemp -d /tmp/uv-22826-repro-XXXXXX)"
mkdir -p "$repro"/{fixture,home,config,data,cache,python,bin,tmp}
printf '3.14.9\n' > "$repro/fixture/.python-versions"
cat > "$repro/fixture/probe.py" <<'PYTHON'
# /// script
# requires-python = ">=3.12"
# ///
import sys
print(sys.version.split()[0])
PYTHON
cd "$repro/fixture"
env -i PATH="$PATH" HOME="$repro/home" TMPDIR="$repro/tmp" \
  XDG_CONFIG_HOME="$repro/config" XDG_DATA_HOME="$repro/data" \
  UV_CACHE_DIR="$repro/cache" UV_PYTHON_INSTALL_DIR="$repro/python" \
  UV_PYTHON_BIN_DIR="$repro/bin" UV_LOCKED=1 UV_NO_PROGRESS=1 \
  uv run probe.py
```

Observed exit code: **2**. No script stdout. Stderr:

```text
warning: No lockfile found for Python script (ignoring `UV_LOCKED=1`); run `uv lock --script` to generate a lockfile
error: No interpreter found for Python 3.14.9 in virtual environments, managed installations, or search path

hint: uv embeds available Python downloads and may require an update to install new versions. Consider retrying on a newer version of uv.
```

The missing-lockfile warning is incidental to the minimal fixture and appears in both successful controls too.

### Controls and catalog inspection

Explicit separate invocations with the same installed uv 0.13.4 produced:

| Configuration | Command | Observed result |
| --- | --- | --- |
| Pin `3.14.9`, default catalog | `uv run probe.py` | Exit 2, exact reported error and update hint |
| Previous pin `3.14.8`, default catalog, fresh installation directory | `uv run probe.py` | Downloads CPython 3.14.8, prints `3.14.8`, exit 0 |
| Pin `3.14.9`, `UV_PYTHON_DOWNLOADS_JSON_URL` set to a temporary copy of `crates/uv-python-managed/download-metadata.json`, fresh installation directory | `uv run probe.py` | Downloads CPython 3.14.9, prints `3.14.9`, exit 0 |
| Default catalog | `uv python list 3.14.9 --all-versions --only-downloads --output-format json` | `[]`, exit 0 |
| Checkout catalog supplied through `UV_PYTHON_DOWNLOADS_JSON_URL` | Same list command | Includes `cpython-3.14.9-linux-x86_64-gnu`, exit 0 |

The catalog override was the only interpreter-selection setting changed for the successful 3.14.9 run. The checkout catalog points to python-build-standalone build `20261021`. Comparing the old and new pins follows the actual configuration change in astral-sh/uv#22802; no alternative uv executable was needed.

Reading `https://raw.githubusercontent.com/astral-sh/versions/main/v1/uv.ndjson` independently found one 0.13.4 record, including its Linux x86_64 artifact, and zero 0.13.5 records. setup-uv v10.3.0 source raises the reported artifact-not-found message when its manifest lookup returns no artifact. The complete setup-uv action, publication scripts, and documentation build were not executed locally. The directly reproduced behavior is the interpreter failure, alongside the observed live manifest omission; end-to-end publication remains untested.

### Existing test coverage

The following tests were read, including setup and assertions, but were not run:

- `crates/uv/tests/project/run.rs::run_pep723_script_requires_python_compatible`: with Python 3.11 and 3.12 available, a script requiring >=3.11 uses the compatible `.python-version` pin to 3.11. This covers the pin precedence relevant to the publishing script.
- `crates/uv/tests/python/venv.rs::create_venv_reads_request_from_python_versions_file`: first selects PATH's Python 3.11, then selects 3.12 after writing a `.python-versions` file listing 3.12 before 3.11. This covers first-entry selection from the plural version file.
- `crates/uv/tests/python/python_list.rs::python_list_remote_python_downloads_json_url`: supplies a mock external catalog and snapshots its interpreters and URLs, plus HTTP, JSON, and digest errors. This covers external catalog loading, not release publication.
- `crates/uv/tests/sync/sync.rs::sync_python_missing_download_hint`: requests unavailable Python 3.100 in a project and snapshots exit 2, the missing-interpreter error, and the same embedded-catalog update hint.

Searches in `crates/uv/tests/` and `crates/uv-client/tests/it/` found no test specifically covering the `publish-versions` workflow or missing `uv.ndjson` release records. These component tests do not establish coverage of the release orchestration failure. No tests were added or checkout files changed.

## Related

- astral-sh/uv#22810 — Use checked-in Python download metadata for releases (merged pull request). Adds `UV_PYTHON_DOWNLOADS_JSON_URL` to release planning, wheel-content checks, and signed-artifact assembly. Its description explicitly notes that reusable workflows need their own setting. The diff omits both publishing workflows; this is the closest prior fix, not a completed fix for manifest publication.
- astral-sh/uv#22802 — Update to include Python 3.15.1, 3.14.9, 3.13.17 (merged pull request). Changes the first `.python-versions` entry from 3.14.8 to 3.14.9 and adds the new download catalog entries. Several checking workflows receive the catalog setting, but the publishing workflows do not. This is the triggering configuration change.
- astral-sh/uv#22793 — uv 0.13.5 listed in CHANGELOG but release appears to have been canceled (closed issue). A maintainer confirmed the earlier run was intentionally canceled because of a Python miscompilation. The current report concerns a later run that published the release but failed manifest and documentation publication.

## Supporting release evidence

- The [0.13.5 GitHub release](https://github.com/astral-sh/uv/releases/tag/0.13.5) was published at 2026-10-22 01:35:54 UTC; the public API confirms it is neither a draft nor a prerelease.
- [Release run 39703477566](https://github.com/astral-sh/uv/actions/runs/39703477566) ran against commit `46776a95d6686ac0160c10611327ce491e6b202f`. The first attempt completed with failure while GitHub publication succeeded.
- The current job API confirms that [manifest publication](https://github.com/astral-sh/uv/actions/runs/39703477566/job/119588017202) failed at "Update versions" after successful "Install uv"; "Commit versions", "Create Pull Request", and "Merge Pull Request" were skipped. Recorded job logs show uv 0.13.4 and the same interpreter error reproduced above.
- The API also confirms that [documentation publication](https://github.com/astral-sh/uv/actions/runs/39703477566/job/119588017234) failed at "Build docs" after successful "Install uv". Recorded job logs show uv 0.13.4 and `No interpreter found for Python 3.14.9 in managed installations or search path`. This documentation command was not reproduced locally.
- At the release commit, [publish-versions.yml](https://github.com/astral-sh/uv/blob/46776a95d6686ac0160c10611327ce491e6b202f/.github/workflows/publish-versions.yml#L58) pins uv 0.13.4 and runs `uv run astral-versions/scripts/convert-cargo-dist-plan.py` piped into `uv run astral-versions/scripts/insert-versions.py --name uv`. [publish-docs.yml](https://github.com/astral-sh/uv/blob/46776a95d6686ac0160c10611327ce491e6b202f/.github/workflows/publish-docs.yml#L81) pins the same uv and runs `uv run --only-group docs mkdocs build --strict -f mkdocs.yml`. Neither sets the catalog override. The local checkout has the same relevant settings.
- The caller's [release.yml](https://github.com/astral-sh/uv/blob/46776a95d6686ac0160c10611327ce491e6b202f/.github/workflows/release.yml#L35) sets the override, but reusable workflows do not inherit that environment, as explained in astral-sh/uv#22810. Both related PR descriptions and relevant diffs were rechecked for this reproduction.
- astral-sh/uv#22802 merged at 2026-10-21 22:37:22 UTC; astral-sh/uv#22810 merged at 2026-10-22 00:56:21 UTC, before the reported release run started at 01:03:53 UTC. The 0.13.5 changelog documents the new Python versions; the local catalog comparison confirms the 0.13.4 executable lacks 3.14.9.

## Recommended next step

Configure both reusable publishing workflows to use the checkout's Python download metadata, following astral-sh/uv#22810. The local control verifies that this lets the pinned uv 0.13.4 run a script with Python 3.14.9. A release maintainer should then complete manifest and documentation publication and verify that setup-uv resolves 0.13.5. Full publishing success is not established by the interpreter-only control.

Keeping setup-uv pinned to 0.13.4 is a temporary workaround for the missing manifest entry. No workflow fix, rerun, publication, GitHub modification, or repository edit was performed during reproduction; only this handoff and temporary reproduction artifacts were written.

## Search scope and excluded candidates

The issue-context investigation searched open and closed issues and PRs for the missing manifest/setup-uv failure, interpreter failure, and documentation failure. Searches included 0.13.5, uv.ndjson, astral-sh/versions, publish-versions, publish-docs, 3.14.9, both reported errors, release publication, missing artifacts, interpreter discovery, Python metadata, and internal:releases. PR keyword searches were supplemented with 600 recent PRs, an open-PR listing, and publish-versions file history. No tracker or fix for these remaining publishing failures was identified; search limitations prevent an exhaustive absence claim. This broader search was not repeated during reproduction.

Excluded candidates remain astral-sh/uv#18701 (manifest proxy/TLS failure), astral-sh/uv#18913 and astral-sh/uv#18917 (missing catalog entries in a shipped release), and astral-sh/uv#18813 (dry-run dist-manifest synthesis). Same-mechanism fixes astral-sh/uv#22633 (test and bug-workflow setup), astral-sh/uv#22816 (Cargo lockfile checking), and astral-sh/uv#22820 (code-quality review setup) do not edit the affected workflows. astral-sh/uv#22745 addresses an earlier 3.14.8 mismatch in signed-release assembly; astral-sh/uv#19788 concerns scheduling uv releases around Python releases.
