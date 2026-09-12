# benchmark

Benchmarking scripts for uv and other package management tools.

## Getting Started

From the `scripts/benchmark` directory:

```shell
uv run resolver \
    --uv-pip \
    --poetry \
    --benchmark \
    resolve-cold \
    ../requirements/trio.in
```

## Installed-sidecar qualification

`qualify-installed-sidecars.py` installs a real wheel from a dependency-free PEP 517 source archive
and the pinned
[Click 8.4.2 wheel](https://files.pythonhosted.org/packages/fb/e2/79c688af8b210d232694e31e59da9f6ec747bae31c3f5946e4e9b98860d5/click-8.4.2-py3-none-any.whl).
It distinguishes absent and malformed `uv_cache.json`/`uv_build.json` from filesystem read failures
and required core metadata. The source backend embeds its build settings in the installed module, so
changed settings, actual build calls, reinstall work, and required artifact hashes are observable
without timing assumptions. The same source archive is also installed through a local flat index to
exercise registry-installed version preferences and the no-preference `--upgrade` path. Input
artifacts are hash-checked and installed files are replaced atomically when constructing a
malformed-metadata case.

```shell
python3 scripts/benchmark/qualify-installed-sidecars.py \
  --uv /path/to/uv --python /path/to/python3.12 \
  --wheel /path/to/click-8.4.2-py3-none-any.whl \
  --expect strict --output installed-sidecars.json
```

The interpreter must be Python 3.12.11. `strict` describes a reader that rejects malformed optional
JSON. `ignore-json-errors` is the negative control that converts every JSON decoder error to missing
metadata, including read-time I/O errors. `invalidate-malformed` requires value-free diagnostics,
propagated I/O errors, and conservative freshness checks for known-corrupt metadata while keeping
the behavior of genuinely absent metadata separate. Use `--case` to select individual controls.
