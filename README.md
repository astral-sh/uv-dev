# uv sync and uv pip install fails for packages with console scripts when the venv contains a `lib` to `usr/lib` symlink

Issue: astral-sh/uv#21255

Classification: bug

## Summary

astral-sh/uv#21255 reports that both `uv sync` and `uv pip install` fail when installing a wheel with a console script into a Unix virtual environment whose `lib` is a symlink to `usr/lib`. The console launcher is written under `<venv>/usr/bin`, but the installer subsequently queries `<venv>/bin` and fails with `failed to query metadata of file ... No such file or directory (os error 2)`.

No existing issue or open pull request tracks the same failure. The closest repository history is astral-sh/uv#19464, a merged entry-point security hardening change. It retained the write through a path relative to `site-packages`, but changed the Unix executable-permission lookup to use the intended absolute scripts path. In the reported symlink layout, traversing `..` from a path below symlinked `lib` can resolve under `usr`, so the write path and permission path refer to different locations.

## Draft response

Thanks for the focused reproduction. This is a bug, and the current installer code matches the behavior you found: the launcher is written using a path relative to site-packages, while the executable-permission step introduced in astral-sh/uv#19464 queries the intended scripts path directly. With `lib` resolving through `usr/lib`, those paths can refer to different locations, so the launcher lands under `usr/bin` and the subsequent lookup under `bin` fails.

The reproduction is sufficient. The next step is to add a Unix integration regression test for this virtual-environment layout and ensure the launcher write and permission update target the same scripts path while preserving the correct `RECORD` entry.

## Classification

This is a bug because a supported install operation writes a console script outside the environment's configured scripts directory and then aborts while looking for that script at the intended location. The current source establishes the relevant inconsistency: `write_script_entrypoints` computes a lexical path from `site_packages` for `write_file_recorded`, while its Unix permission block calls `fs::metadata` on `script.as_path()`.

astral-sh/uv#19464 is historical context rather than a duplicate. Its merged commit changed the permission lookup from `site_packages.join(entrypoint_relative)` to `script.as_path()`, exposing the mismatch as the reported hard failure. Before that change, the permission lookup followed the same symlink-sensitive path as the write, so the installer could complete while still placing the script incorrectly. No open issue or pull request was found that already centralizes this Unix regression.

## Related

- astral-sh/uv#19464 — Merged pull request, “Enforce that entry points cannot escape in the scripts directory.” This is the reporter's cited change and the closest repository evidence. It introduced `ValidatedScript`, retained the site-packages-relative write used for `RECORD`, and switched the Unix permission lookup to the direct scripts path. That difference is exactly what exposes the missing `<venv>/bin/<script>` after the write traverses the symlinked `lib` path.

## Search and supporting evidence

Searches covered the exact `failed to query metadata of file` and `No such file or directory` fragments; `console_scripts`; console-script and entry-point installation; `site-packages`, `RECORD`, and scripts-directory relative paths; `usr/bin`; virtual-environment `lib`, `bin`, and interpreter symlinks; `script.as_path`; and the cited commit. Open and closed issues and open, closed, and merged pull requests were included, with version-specific merged history checked because the report is against uv 0.12.0.

The uv 0.12.0 release was published after astral-sh/uv#19464 merged, and current `main` retains the relevant split between the site-packages-relative write and direct scripts-path metadata lookup. The pull request has no linked public issue or follow-up discussion identifying another canonical tracker.

The strongest adjacent issues were inspected and ruled out:

- astral-sh/uv#18728 concerns `uv pip list` not following symlinked `.dist-info` directories inside `site-packages`; it does not involve installing entry points or a path escaping through a symlinked parent directory.
- astral-sh/uv#19374 concerns Windows native console launchers selecting a base interpreter in a stdlib venv created with `--symlinks`; it is a launcher-execution problem after installation, not this Unix launcher-placement and metadata-lookup failure.
- astral-sh/uv#11048 concerned a generated shebang using a resolved symlink target instead of `sys.executable`; it was fixed by astral-sh/uv#11083 and involves interpreter discovery rather than the destination used to write a launcher.
- astral-sh/uv#15800 requests optional cross-platform `bin`/`Scripts` symlinks, and astral-sh/uv#5152 concerned overwriting an existing interpreter symlink with a reserved script name. Neither shares this trigger or mechanism.
