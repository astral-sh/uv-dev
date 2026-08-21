# uv sync and uv pip install fails for packages with console scripts when the venv contains a `lib` to `usr/lib` symlink

Issue: astral-sh/uv#21255

Classification: bug

## Summary

astral-sh/uv#21255 reports that both `uv sync` and `uv pip install` fail when installing a wheel with a console script into a Unix virtual environment whose `lib` is a symlink to `usr/lib`. A targeted reproduction confirms the report with uv 0.12.0 and the installed uv 0.12.5 on Ubuntu: the launcher is written under `<venv>/usr/bin`, then uv queries `<venv>/bin` and fails with `No such file or directory (os error 2)`.

No pre-existing uv issue or production fix pull request tracks the same failure. The parent regression test is astral-sh/uv-dev#831. The closest repository history is astral-sh/uv#19464, a merged entry-point security hardening change. It retained the write through a path relative to `site-packages`, but changed the Unix executable-permission lookup to use the intended absolute scripts path. In the reported symlink layout, traversing `..` from a path below symlinked `lib` can resolve under `usr`, so the write path and permission path refer to different locations.

## Reproduction

Outcome: **reproducible** on Ubuntu Linux x86_64 with Python 3.12.3. The installed executable was uv 0.12.5; uv 0.12.0 and uv 0.11.1 were also compared using isolated tool and cache directories under `/tmp`.

The issue's shell snippet cannot be run literally because `uv venv` creates `<venv>/lib`, so its later `ln -s usr/lib <venv>/lib` fails with `File exists`. The stated filesystem layout can be reconstructed by moving that directory first:

```console
$ uv venv --python python3.12 "$V"
$ mkdir -p "$V/usr"
$ mv "$V/lib" "$V/usr/lib"
$ ln -s usr/lib "$V/lib"
$ mkdir -p "$V/usr/bin"
$ VIRTUAL_ENV="$V" uv pip install demo_script-1.0-py3-none-any.whl
Using Python 3.12.3 environment at: .../venv
Resolved 1 package in 1ms
Prepared 1 package in 2ms
error: Failed to install: demo_script-1.0-py3-none-any.whl (...)
  Caused by: failed to query metadata of file `.../venv/bin/demo-script`: No such file or directory (os error 2)
```

`demo_script-1.0-py3-none-any.whl` was a locally generated minimal pure-Python wheel whose only entry point was `demo-script = demo_script:main`. After the failure, `<venv>/bin/demo-script` did not exist and `<venv>/usr/bin/demo-script` did exist. This avoids relying on the report's external package while exercising the same wheel installation path.

`uv sync` was reproduced separately with a non-packaged project depending directly on the same local wheel. Its pre-created `.venv` used the same `lib -> usr/lib` layout. uv 0.12.5 exited 2 with the same metadata error for `.venv/bin/demo-script`, while the launcher existed at `.venv/usr/bin/demo-script`.

The version comparison used the same wheel and venv layout:

- uv 0.11.1 exited 0, but placed the launcher in `<venv>/usr/bin` rather than `<venv>/bin`.
- uv 0.12.0 exited 2 with the reported metadata error; the launcher was still present in `<venv>/usr/bin` and absent from `<venv>/bin`.
- uv 0.12.5 also exited 2 with the same result.

Before the parent regression was added, no integration test covered the internal `lib -> usr/lib` venv layout. `crates/uv/tests/pip_install/pip_install.rs::launcher_with_symlink` installs a launcher normally and then symlinks that launcher outside the environment. `crates/uv/tests/sync/sync.rs::sync_virtual_env_warning` exercises a symlink to the whole environment, and the entry-point tests near `reject_wheel_entrypoint_paths` exercise lexical entry-point names. None of those earlier tests exercises a symlinked site-packages parent causing the recorded relative path and scripts destination to resolve differently.

## Fix

Outcome: **fixed in the checkout**.

The reproduction and implementation confirm that paths stored relative to `site-packages` were computed from the unresolved `lib/python3.12/site-packages` spelling. The operating system resolves `lib -> usr/lib` before processing later `..` components, so the relative launcher path reached `usr/bin` rather than `bin`. The same assumption affected the separate wheel `.data/scripts` producer: it wrote directly to `bin`, but recorded a relative path that caused uninstall to look under `usr/bin` and leave the installed script behind.

`crates/uv-install-wheel/src/wheel.rs` now resolves the site-packages base before computing RECORD-relative paths for both generated entry points and wheel data scripts. `crates/uv-install-wheel/src/uninstall.rs` validates RECORD paths from that same resolved base, preserving the traversal check while making install and uninstall interpret parent components consistently.

The parent `install_script_with_symlinked_lib` regression was changed from asserting the failure to asserting a successful install into `<venv>/bin`, absence from `<venv>/usr/bin`, and successful removal through `uv pip uninstall`. The existing `install_editable_uv_build_data` integration test now uses the symlinked layout on Unix and verifies that its independently produced data script is removed on uninstall. Before the second production correction, this test demonstrated the distinct failure by leaving `project-script` in `bin` after an otherwise successful uninstall.

Focused debug-profile validation passed for the parent regression, the editable wheel data-script round trip, normal and relocatable launchers, the existing malicious RECORD traversal integration test, and the corresponding `uv-install-wheel` unit test. Focused `uv-install-wheel` Clippy passed with warnings denied. The changed Rust files were formatted with the available stable rustfmt because the pinned 1.97.1 toolchain's rustfmt component could not be installed in the read-only toolchain directory.

## Classification

This is a bug because the observed install operation writes a console script outside the environment's configured scripts directory and then aborts while looking for that script at the intended location. The pre-fix `write_script_entrypoints` implementation explains the observation: it computed a path from the unresolved `site_packages` location for `write_file_recorded`, while its Unix permission block called `fs::metadata` on `script.as_path()`.

The version comparison supports the reported regression: uv 0.11.1 completed despite the incorrect placement, while uv 0.12.0 and later failed after writing to the same incorrect location. astral-sh/uv#19464 is historical context rather than a duplicate. Its merged change to the permission lookup is consistent with that boundary. The fix investigation confirmed the underlying unresolved-base mismatch by correcting it and exercising both entry-point and wheel data-script install/uninstall round trips.

## Related

- astral-sh/uv-dev#831 — Parent pull request that added the original passing regression test asserting the undesirable console-entry-point failure.
- astral-sh/uv#19464 — Merged pull request, “Enforce that entry points cannot escape in the scripts directory.” This is the reporter's cited change and the closest repository evidence. It introduced `ValidatedScript`, retained the site-packages-relative write used for `RECORD`, and switched the Unix permission lookup to the direct scripts path. That difference is exactly what exposes the missing `<venv>/bin/<script>` after the write traverses the symlinked `lib` path.

## Search and supporting evidence

Searches covered the exact `failed to query metadata of file` and `No such file or directory` fragments; `console_scripts`; console-script and entry-point installation; `site-packages`, `RECORD`, and scripts-directory relative paths; `usr/bin`; virtual-environment `lib`, `bin`, and interpreter symlinks; `script.as_path`; and the cited commit. Open and closed issues and open, closed, and merged pull requests were included, with version-specific merged history checked because the report is against uv 0.12.0.

The uv 0.12.0 release was published after astral-sh/uv#19464 merged, and the pre-fix implementation retained the relevant split between the site-packages-relative write and direct scripts-path metadata lookup. The pull request has no linked public issue or follow-up discussion identifying another canonical tracker.

The strongest adjacent issues were inspected and ruled out:

- astral-sh/uv#18728 concerns `uv pip list` not following symlinked `.dist-info` directories inside `site-packages`; it does not involve installing entry points or a path escaping through a symlinked parent directory.
- astral-sh/uv#19374 concerns Windows native console launchers selecting a base interpreter in a stdlib venv created with `--symlinks`; it is a launcher-execution problem after installation, not this Unix launcher-placement and metadata-lookup failure.
- astral-sh/uv#11048 concerned a generated shebang using a resolved symlink target instead of `sys.executable`; it was fixed by astral-sh/uv#11083 and involves interpreter discovery rather than the destination used to write a launcher.
- astral-sh/uv#15800 requests optional cross-platform `bin`/`Scripts` symlinks, and astral-sh/uv#5152 concerned overwriting an existing interpreter symlink with a reserved script name. Neither shares this trigger or mechanism.

Pull request: https://github.com/astral-sh/uv-dev/pull/832
