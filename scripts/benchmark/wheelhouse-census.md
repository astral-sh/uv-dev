# Ordinary local wheelhouse census

This is a fixture generator and ordinary-I/O census, not a production optimization or an `io_uring`
benchmark. Its production reference is commit `cdc71a4fd8cd5aa76304c5af6e13473488283f62`, tree
`a6dca5e1aba65d19d0c0154321484db4b28a7449`.

`FlatIndexClient::read_from_directory` performs a no-follow metadata lookup before it parses each
distribution filename. The first question is whether those lookups account for a meaningful part of
a real offline `uv pip compile` command. Directory enumeration, filename parsing, resolver work, and
reading the selected wheel are separate costs.

## Prepare the cardinality controls

Use an isolated source checkout, Cargo target directory, and Cargo build directory. No release
profile, package download, native timing, or existing uv cache is needed to prepare the fixtures.
For example, with absolute, task-owned paths:

```sh
env CARGO_TARGET_DIR="$WHEELHOUSE_TARGET" \
  cargo --config "build.build-dir=\"$WHEELHOUSE_BUILD\"" \
  build -p uv-test --example wheelhouse_catalog --locked --offline

env CARGO_TARGET_DIR="$WHEELHOUSE_TARGET" \
  cargo --config "build.build-dir=\"$WHEELHOUSE_BUILD\"" \
  test -p uv-test --test wheelhouse_classification --locked --offline

python3 scripts/benchmark/wheelhouse_census.py prepare \
  --generator "$WHEELHOUSE_TARGET/debug/examples/wheelhouse_catalog" \
  --generator-sha256 "$WHEELHOUSE_GENERATOR_SHA256" \
  --stage "$WHEELHOUSE_NEW_STAGE"
```

The stage must not exist. The helper calls the pinned `uv_test::packse::generate_wheel` once for
each distinct distribution, then writes identical bytes to the applicable catalogs. The three
wheelhouses contain 1, 1,000, and 10,000 valid, no-dependency `py3-none-any` wheels, with
`uv-census-target==1.0.0` as the sole requested package. These are synthetic catalog-cardinality
controls, not observations of deployed wheelhouses.

Preparation checks every wheel's filename, four unique regular ZIP members, stored sizes and CRCs,
`METADATA`, `WHEEL`, and exact independently reconstructed SHA-256 `RECORD`. The Rust helper also
parses the embedded identity and compatibility tags with uv's actual types. It returns an immutable
`catalogs.json` path and SHA-256. Each catalog has a sorted entry manifest and aggregate digest. A
failed preparation leaves its new stage available for inspection; no directory is reused or removed
automatically.

```sh
python3 scripts/benchmark/wheelhouse_census.py verify \
  --manifest "$WHEELHOUSE_MANIFEST" \
  --manifest-sha256 "$WHEELHOUSE_MANIFEST_SHA256"
```

The portable behavior tests compare a metadata-based projection and a type-only projection against
the public production `FlatIndexClient::fetch_all`. They cover sorting, ignored names and
directories, no-follow symlink and special-file handling, errors before filename filtering, and
retained-`DirEntry` deletion. The latter records the real API's filesystem-dependent result and
separately injects a saved type to demonstrate the freshness distinction deterministically. They do
not change production classification.

## Native ordinary-I/O capture

Schedule this only when the native worker is idle. Use the saved reference CLI from
`7f42434960c790fb70736558fddb4613c9fbb833`, tree `a8fcfbe60aa504bb62c9ba4a7a7e56bc8d9b397c`, SHA-256
`1e5dee79bdb92a4b4cb60bb99119455ab5ced5585b82a0ddcdb2288a7e2023df`, or an independently attributed
CDC binary. The relevant production files are unchanged between those references. The executable,
interpreter, and `strace` paths and hashes are required inputs, not inferred from a filename.

```sh
python3 scripts/benchmark/wheelhouse_census.py capture \
  --manifest "$WHEELHOUSE_MANIFEST" \
  --manifest-sha256 "$WHEELHOUSE_MANIFEST_SHA256" \
  --uv "$WHEELHOUSE_UV" --uv-sha256 "$WHEELHOUSE_UV_SHA256" \
  --uv-source "$WHEELHOUSE_UV_SOURCE" --uv-tree "$WHEELHOUSE_UV_TREE" \
  --python "$WHEELHOUSE_PYTHON" --python-sha256 "$WHEELHOUSE_PYTHON_SHA256" \
  --strace "$WHEELHOUSE_STRACE" --strace-sha256 "$WHEELHOUSE_STRACE_SHA256" \
  --run-id ordinary-01
```

Every child gets a cleared environment, `--offline`, `--no-index`, `--only-binary :all:`, an
explicit interpreter with downloads disabled, and fresh task-owned cache and temporary directories.
Each fixture receives one first-use uv-cache run, three untraced warm-cache fresh processes, one
`strace -c` summary, and one pathname trace. The driver records stdout, stderr, status, argv,
environment, elapsed time, tool identities, raw traces, and pathname-attributed syscall counts. It
rejects changed catalog bytes, nonzero exit statuses, unequal expected stdout, and incomplete trace
parsing. It never interprets traced syscall time as a speedup. Fixture verification warms OS pages;
a first-use uv cache is not a cold-filesystem-cache result.

The trace filter is `openat,read,close,getdents64,statx,newfstatat,readlink,readlinkat`. The
selected-wheel read counts cover only this filter's `read` calls; they do not measure every possible
I/O operation, such as `pread64` or memory-mapped reads. The standalone `attribute` command can also
inspect an independently collected real-wheelhouse trace. Supply that child's actual CWD so relative
`AT_FDCWD` paths are resolved correctly.

## Real fixtures and the next decision

Keep two possible real catalogs distinct:

- The preserved 93-distribution Jupyter environment does not prove that all original wheel ZIPs
  remain. A recovered catalog needs per-wheel original/repacked provenance, strict original `RECORD`
  validation, immutable byte hashes, the pinned 93-package requirements, and its own package-set
  oracle. Missing wheels end that real row.
- The public 96-wheel Jupyter 1.1.1 catalog associated with
  [astral-sh/uv#21619](https://github.com/astral-sh/uv/pull/21619) is a different fixture. Its exact
  manifest, pins, requirements, and original wheel bytes must already be available and verified. Do
  not invoke its download helper as an inventory operation.

This preparer accepts only its generated synthetic manifest. It does not relabel generated wheels as
real, reconstruct cached wheels, fetch artifacts, or infer a complete dependency closure. Apply the
separately pinned real-fixture census plan once the necessary bytes exist.

Proceed to an ordinary `metadata` versus `file_type` control only if the command census shows
meaningful linear type-lookup cost. Compare full accepted-entry projections before timing; keep
directory enumeration, in-memory parsing/sorting, public `fetch_all`, and the complete CLI as
separate denominators. A prospective production change needs a representative full-command
improvement of at least 5% and 1 ms, plus an explicit decision about deleted-entry freshness. Stop
if the portable control removes the meaningful metadata work or another phase dominates.

The adjacent selected-wheel reader change in astral-sh/uv#21619 was merged as
`6e5250cecdf640e4adaaeccdbf43301042ffb602`. A later production A/B must include that same reader on
both arms, or use a predeclared two-by-two comparison. Its gain must not be attributed to directory
classification. There is no ring backend in this harness.
