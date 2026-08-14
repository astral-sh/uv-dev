# `build::venv_included_in_sdist` test fails in 0.12.4 release

Issue: astral-sh/uv#21128

Classification: bug

## Summary

The failure reported on Gentoo Linux amd64 is reproducible with uv 0.12.4. When a source distribution contains `.venv/bin/python` as an absolute symlink to an interpreter laid out as `.../python/3.12/python3`, both extraction backends correctly reject the invalid archive, but the preview `tar-codec` backend omits the intended virtual-environment hint. The legacy backend includes the hint.

A control using an interpreter target laid out as `.../bin/python3` makes the 0.12.4 `tar-codec` backend print the hint. This confirms that the diagnostic depends on the interpreter installation path, matching the Gentoo test failure.

## Classification

This is a reproducible platform-sensitive diagnostic bug. The archive rejection is expected; the bug is that the preview extraction path loses the explanatory hint and therefore fails the committed snapshot in `crates/uv/tests/build/build.rs`, test `venv_included_in_sdist`.

The implementation in `crates/uv/src/commands/build_frontend.rs` classifies a tar-codec unsafe symlink as a virtual-environment interpreter only when the symlink target itself has a parent ending in `bin` and a filename beginning with `python`. The reported and reproduced target ends in `python/3.12/python3`, so it is not recognized even though the archive member is `.venv/bin/python`.

## Reproduction

Outcome: **reproducible**.

Environment used:

- Ubuntu 24.04, Linux x86_64
- installed `uv 0.12.4 (x86_64-unknown-linux-gnu)`
- CPython 3.12.3
- all project files, tool installations, Python copies, and uv caches under `/tmp`

The minimal project used Hatchling and deliberately included `.venv` in its sdist:

```toml
[project]
name = "project"
version = "0.1.0"
requires-python = ">=3.12.0"

[tool.hatch.build.targets.sdist.force-include]
".venv" = ".venv"

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"
```

After adding `src/project/__init__.py`, the relevant setup and commands were:

```console
$ mkdir -p "$CASE/python/3.12"
$ cp /usr/bin/python3.12 "$CASE/python/3.12/python3"
$ uv venv --no-config --python "$CASE/python/3.12/python3" --clear .venv
$ readlink .venv/bin/python
/tmp/.../python/3.12/python3
$ uv build --no-config
...
Caused by: symlink path `/tmp/.../python/3.12/python3` is absolute, but external symlinks are not allowed

hint: The source distribution includes a virtual environment. Virtual environments must be excluded from source distributions.
$ uv build --no-config --preview-features tar-codec
...
Caused by: at byte 41472: unsafe symbolic-link target "/tmp/.../python/3.12/python3": is absolute
```

The second command exited with status 2 and omitted the hint, reproducing the reported snapshot difference. With the same fixture but an interpreter copied to `/tmp/.../bin/python3`, the preview command still exited with status 2 and did print the hint.

For comparison, uv 0.12.3 rejected the same fixture through the legacy extractor and printed the hint. It warned that `tar-codec` was an unknown preview feature, so the affected preview path was not available in that release.

Existing integration coverage is `crates/uv/tests/build/build.rs`, test `venv_included_in_sdist`. It constructs a Hatchling project that force-includes `.venv`, verifies rejection by both extractors, and snapshots the hint for both. Its interpreter fixture normally resolves to a `bin/python*` target, so it does not cover the reproduced `python/3.12/python3` layout and fails rather than passes on Gentoo.

## Related

- astral-sh/uv#15096 requested the explanatory hint for virtual environments included in source distributions.
- astral-sh/uv#15202 added the hint and the original integration coverage.
- astral-sh/uv#19979 introduced the preview tar-codec extraction path, added its structured unsafe-link hint detection, and added the second snapshot shortly before uv 0.12.4.

## Maintainer handoff

The observed issue is limited to hint detection; both extractors continue to reject the invalid sdist. Any fix should preserve the structured tar-codec checks while recognizing the archive member `.venv/bin/python` independently of the host interpreter's absolute installation layout. Coverage should include a symlink target shaped like `.../python/3.12/python3` as well as the existing `.../bin/python*` shape.
