# Direct Rust AST translation experiment

This experiment mechanically translates **26 selected Rust functions** from `uv`
into ordinary Python. Its public backend connects 13 naming, serialization,
entry-point, default-setting, and wheel-header functions to handwritten packaging
adapters. Other emitted functions include identifiers,
classifiers, import names, and the bodies of metadata and license conversion.
The latter still depend on untranslated libraries and raise
`UntranslatedDependency` at that boundary.

**This route does not meet full parity.** The complete compiled-program route is
in the adjacent `uv-build-slim-wasm` directory. That route includes the Rust
backend's dependency closure; this experiment demonstrates the extra work needed
for a direct source translation. Its installed runtime uses only the Python
standard library, with Rust needed for regeneration and reference tests.

The reference is the clean `uv` checkout at
[`bd39416351244f273aeff89d495d4b0804b765a5`](https://github.com/astral-sh/uv/commit/bd39416351244f273aeff89d495d4b0804b765a5),
whose backend version is `0.13.0`. This is a pinned development revision, not a
claim that every file matches the published `0.13.0` release.

## What is generated

`codegen/` uses `syn` to parse unchanged, vendored Rust source into a closed AST.
Its Python emitter generates ordinary Python functions with branches, loops,
pattern matches, closures, and calls. It does not rewrite source with regular
expressions, copy Python templates for individual Rust functions, call an LLM, or
invoke the Rust backend at runtime.

| Rust implementation | Generated behavior |
| --- | --- |
| `validate_and_normalize_ref`, `is_normalized`, `normalize` | Package-name validation and normalization |
| `PackageName::as_dist_info_name` | Distribution-name escaping |
| `InvalidNameError::fmt` | Exact normalization error messages |
| `Metadata23::core_metadata_format` | Metadata header order, optional and repeated fields, line folding, description body |
| `Keywords::as_metadata`, `ProjectUrls::to_vec_str` | Metadata formatting |
| `PyProjectToml::to_entry_points`, `PyProjectToml::write_group` | Entry-point ordering, validation, errors, and text |
| `BuildBackendSettings::default` | Backend defaults |
| `WheelInfo::new`, `WheelInfo::fmt` | `WHEEL` contents |
| `Identifier` methods, `Classifier` methods, import-name parsing | Generated source outside the public backend integration |
| `PyProjectToml::to_metadata`, `PyProjectToml::license_metadata` | Generated bodies with unresolved PEP 508, glob, SPDX, and filesystem dependency calls |

The generator also derives struct layouts and error templates from Rust. It emits
the actual Rust Unicode alphanumeric table, so Python's Unicode version does not
change the entry-point character predicate. The mutable string, option, iterator,
UTF-8 byte indexing, and line-splitting adapters are explicit in `_runtime.py`.

The handwritten `_backend.py` contains TOML-to-model conversion, supported-domain
validation, filesystem operations, file selection, ZIP writing, and `RECORD`
hashing. Those adapters remain part of the correctness boundary. This is a
vertical slice of the backend, not a translation of its entire dependency graph.

## Parity contract and limits

**Code generation alone does not guarantee exact parity.** This PoC provides
reproducible translation plus differential evidence, not a formal proof. The
compiler, handwritten adapters, and Python standard library could contain
differences that the current tests have not exposed.

The tests compare generated functions directly with compiled upstream Rust. End
to end, they compare wheel filenames, ordered member lists, every uncompressed
member byte (including `WHEEL`, `METADATA`, entry points, and `RECORD`), Unix modes,
and compression methods. Prepared metadata and editable wheels are covered.
Compressed archive bytes, ZIP header representation, diagnostic wrappers,
tracing, allocation failures, and performance are outside this contract.

Unknown Rust syntax, methods, and external symbols stop generation. Selected
source files are pinned by SHA-256. Unsupported project features raise
`UnsupportedOperation`; they are not silently approximated. A source mutation
test verifies that changing a Rust header literal changes generated behavior.
Another test verifies that introducing an unknown Rust macro fails generation.
The checked-in `parity-report.json` records the latest run.

Proving equivalence for all accepted inputs would additionally require a typed,
semantics-preserving lowering with verified runtime adapters, a precise observable
behavior specification, and closure over all called implementations. Translating
the full backend also requires the PEP 440/508 parsers, glob semantics, SPDX
validation, filesystem and archive writers, and sdist TOML rewriting. The direct
generator does not expand Rust macros, monomorphize traits and generics, or
translate the entire dependency closure.

## Supported package inputs

- Python 3.11 or newer on POSIX.
- Numeric canonical release versions, such as `1.2.3`.
- Ordinary source or flat layouts, one explicit dotted module, and stub packages.
- Normalized distribution names; ASCII module identifiers.
- UTF-8 summaries and readmes, legacy text/file licenses, authors, maintainers,
  keywords, project URLs, console/GUI scripts, and custom entry-point groups.
- A single canonical `requires-python` comparison, such as `>=3.11`.
- Default bytecode exclusions, optionally disabled; executable file modes.
- `build_wheel`, `build_editable`, and both metadata-preparation hooks.

Dependencies, nonempty classifiers, SPDX licenses, license globs, arbitrary PEP
440/508 syntax, multiple modules, custom include/exclude globs, data directories,
symlinks, preview features, and Windows are rejected. Sdist generation raises
`UnsupportedOperation`, because its TOML serializer and file selection have not
been translated. This package is not ready to replace `uv-build` in a distribution.

## Try it without Rust

From this directory, build the PoC's own wheel using the checked-in Python:

```sh
mkdir -p dist
PYTHONPATH=src python3 -c 'import uv_build_slim; print(uv_build_slim.build_wheel("dist"))'
```

The distribution is named `uv-build-slim`; its import name and build backend are
`uv_build_slim`. It has no runtime dependencies. It is not published to PyPI.

To build the example with this source checkout:

```sh
PYTHONPATH=src python3 -c 'import os; from pathlib import Path; import uv_build_slim; output = Path("dist").resolve(); os.chdir("examples/hello"); print(uv_build_slim.build_wheel(str(output)))'
```

`tools/isolated_smoke.py` additionally imports the built wheel in an isolated
Python process with an empty `PATH`, builds the example, and imports its result.
It verifies that no native binary or source checkout is needed at runtime:

```sh
python3 tools/isolated_smoke.py --wheel dist/uv_build_slim-0.1.0-py3-none-any.whl
```

## Regenerate and validate

Use Rust `1.99.0`, matching `rust-toolchain.toml`. Build artifacts and fixtures can
be kept under `~/code/tmp`:

```sh
export CARGO_TARGET_DIR="$HOME/code/tmp/uv-build-slim-poc/target"
export TMPDIR="$HOME/code/tmp/uv-build-slim-poc/tmp"
mkdir -p "$CARGO_TARGET_DIR" "$TMPDIR"
set -o pipefail
cargo run --locked --manifest-path codegen/Cargo.toml -- vendor |
  python3 codegen/generate.py --ast /dev/stdin
```

Build the native backend from a **clean checkout at the pinned revision**:

```sh
cargo build --locked --manifest-path /path/to/uv/Cargo.toml -p uv-build
python3 tests/build_oracle.py --uv-source /path/to/uv
python3 tests/test_parity.py -v --report parity-report.json
```

`--reference`, `--oracle`, and `--frontend` select alternative binary locations.
The tests require the native executables and fail if they are missing. They do
not substitute another Python implementation as the reference.

The test inputs include randomized names and metadata, Rust Unicode range
boundaries, CR/LF and other Unicode line separators, empty optional values,
project-URL insertion order, CSV quoting, executable files, namespace and stub
layouts, prepared-metadata mismatches, and rejected unsupported features.

Upstream source and generated derivatives retain the upstream MIT/Apache-2.0
licensing; see `LICENSE-MIT`, `LICENSE-APACHE`, and `vendor/manifest.json`.
