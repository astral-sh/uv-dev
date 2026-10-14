# Complete Rust backend to generated Python

This PoC compiles the complete `uv-build` program and its dependencies to
WebAssembly, then translates every instruction in that module into Python basic
blocks. The installed backend runs those generated Python functions using only
the Python standard library. It does not execute a native backend, load a Wasm
engine, or import `pywasm` at runtime.

The reference is `astral-sh/uv` at
[`bd39416351244f273aeff89d495d4b0804b765a5`](https://github.com/astral-sh/uv/commit/bd39416351244f273aeff89d495d4b0804b765a5),
with backend version `0.13.0`. This is a pinned development revision. The generated
Unix module contains **7,157 functions and 846,377 translated instructions**. The
distribution is named `uv-build-slim-wasm`; it provides both `uv_build` and
`uv_build_slim_wasm` imports and all eight PEP 517/660 hooks.

**The complete backend is translated, but unrestricted full parity is not
established.** Native Linux and macOS comparisons pass. Separate Windows programs
retain the Windows standard library and call Windows APIs through `ctypes`;
the native Windows CI matrix targets 32-bit, x64, and ARM64 Python. The Unix program still has
a 32-bit address space. Finite differential tests are not a proof of
equivalence for every input and environment. This remains an experimental local
package, with substantial runtime overhead.

## What is included

The Rust implementations of TOML parsing and rewriting, PEP 440/508, SPDX,
classifiers, import names, metadata, globs, namespace and stub layouts, file
selection, data directories, entry points, wheel and sdist serialization,
compression, hashing, prepared-metadata consistency, editable wheels, and preview
metadata are retained in the compiled dependency closure. There is no separate
Python reimplementation of these packaging rules.

`codegen/generate.py` lowers the module into Python functions. Unknown
instructions or unsupported module constructs stop generation. `_code.bin`
contains compressed UTF-8 Python source, grouped into chunks; `_program.json`
describes functions and memory layout; `_data.bin` contains the Rust program's
static data. `_vm.py` supplies linear memory, calls, and integer operations.
Execution dispatches generated Python blocks, rather than interpreting Wasm
opcodes. Source and data checksums detect damaged payloads.

The portability boundary is explicit:

- `codegen/wasi-port.patch` adds WASI filesystem support, native executable bits,
  and host working-directory handling. The original checkout is unchanged.
- `codegen/prepare_sysroot.py` copies Rust library sources into an isolated
  sysroot and adapts typed I/O error formatting. Internal WASI errno values remain
  unchanged; displayed messages and numbers come from the host operating system.
- `_host.py` and the derived `_wasi_impl.py` implement the filesystem and other
  host imports. The adapter handles UTF-8 byte lengths, non-UTF-8 POSIX paths,
  symlinks, file permissions, and native error reporting.
- `_posix.py` supplies native descriptor APIs for macOS Python builds that omit
  them. Directory operations continue to use open descriptors across renames.
- `codegen/build_windows.py` builds separate 32-bit and 64-bit Windows programs.
  The included patches retain Windows path and filesystem implementations while
  exposing DLL imports to `_host_windows.py`. Each invocation owns its heap,
  handles, thread-local state, and callbacks. The Windows programs contain
  17,120 and 17,086 generated functions respectively.

The Rust compiler, the instruction translator, and these host adapters remain
part of the correctness boundary. `wasm32-wasip1` also changes pointer width,
allocation limits, and some target-specific library code compared with native
64-bit Rust. Concurrency, resource exhaustion, terminal behavior, and filesystem
races have not been proven equivalent.

## Evidence

Each report identifies the payload or source revision it checked. Earlier reports
describe the initial Unix payload; the CI artifacts record the current package.
The checks include:

- All **66 original `uv-build-backend` library tests**, with **62 differential
  hook calls** added around the native implementation. Comparisons cover complete
  archive bytes, filenames, prepared metadata, output modes, failure cleanup,
  success status, and error root text. A mismatch fails the Rust test.
- **15 filesystem and validation edge tests** with exact stderr comparisons,
  including Unicode, non-UTF-8 names and working directories, symlinks, missing
  files, invalid UTF-8 readmes, SPDX, dependency markers, and invalid globs.
- **Six descriptor-level checks** for metadata after unlink, native device and
  inode values, failed seek position, and unsigned descriptor bounds.
- **5,015 memory32 and 5,021 memory64 instruction-level comparisons** against V8's independent WebAssembly
  engine, including integer boundaries, division traps, floating-point rounding,
  typed control flow, indirect calls, and memory operations.
- Native artifact comparisons across **CPython 3.8 through 3.15 on Linux**.
- Native macOS checks on ARM64 and Intel, including the older Intel Python 3.8
  build without Python-level descriptor APIs.
- Windows Python comparisons across supported minor versions and free-threaded
  variants. Wine results and native Windows CI results are recorded separately.
- Concurrent builds with the GIL disabled on Python 3.13, 3.14, and 3.15.
- A wheel-only check that exercises all eight hooks with an empty `PATH`,
  isolated Python imports, and subprocess and network operations blocked.
- A file larger than the upstream **16 MiB ZIP streaming threshold**.

These are functional checks, not controlled performance benchmarks. Timing
reports include cold compilation and sometimes concurrent test processes. The
generated runtime is far slower than the native backend, particularly for large
files, because compression and hashing also execute as generated Python.
Unavailable filesystem fixtures are recorded separately from passing checks.
FreeBSD is excluded from this validation run. See `platform-matrix.json` for
remaining platform coverage.

## Try the checked-in Python

No Rust toolchain is needed to use the checked-in generated backend. From this
directory, build a package whose `pyproject.toml` already uses `uv_build`:

```sh
PYTHONPATH="$PWD/src" python3 -c '
import os
from pathlib import Path
import uv_build
output = Path("dist").resolve()
output.mkdir(exist_ok=True)
os.chdir("/path/to/package")
print(uv_build.build_wheel(str(output)))
'
```

Use `build_sdist`, `build_editable`, or either metadata hook in the same way.
For a wheel-only test, the packaged wheel can be placed directly on `sys.path`;
its generated resources support zip imports. The `uv_build` import is a
replacement entry point, so this experiment should be kept separate from an
installation of the ordinary `uv_build` package.

The PoC's own wheel can be built by calling its checked-in Python directly:

```sh
mkdir -p dist
PYTHONPATH=src python3 -c 'import uv_build; print(uv_build.build_wheel("dist"))'
```

## Regenerate from Rust

Generation uses Python 3.12, Rust 1.99.0 with `rust-src` and `wasm32-wasip1`, and
the pinned `pywasm` parser source at
[`47abba6c94ecf74c3c173344684891f2ca29c2e3`](https://github.com/libraries/pywasm/commit/47abba6c94ecf74c3c173344684891f2ca29c2e3).
`pywasm` is needed only during generation. The WASI provider is derived from its
MIT-licensed source and has no interpreter dependency.

```sh
python3 codegen/build.py \
  --uv-source /path/to/uv \
  --pywasm-source /path/to/pinned/pywasm \
  --work "$HOME/code/tmp/uv-build-slim-regenerate"
python3 codegen/licenses.py --uv-source /path/to/uv
```

`--work` must have no existing `uv` child directory. The script archives the pinned
commit instead of modifying the supplied checkout. It applies the included port,
creates a separate sysroot, builds the `dev` profile with size optimization and
LTO, and generates the Python payload. The unstable WASI/build-std interfaces use
`RUSTC_BOOTSTRAP=1` with the pinned compiler. Build paths are remapped; source,
generator, patch, toolchain, and module provenance is recorded in
`src/uv_build_slim_wasm/_provenance.json`.

Generate either Windows ABI from the same pinned inputs using a new work directory:

```sh
python3 codegen/build_windows.py \
  --uv-source /path/to/uv \
  --pywasm-source /path/to/pinned/pywasm \
  --work "$HOME/code/tmp/uv-build-slim-windows64" \
  --pointer-width 64 --output src/uv_build_slim_wasm/_windows64
```

Use `--pointer-width 32` and `_windows32` for 32-bit Python. Generation verifies
registry archives against the original `Cargo.lock`, applies patches only to
scratch copies, and rejects dependency-version changes. Cargo must already have
the pinned Windows dependencies in its download cache. Runtime installation
does not require Rust, a C compiler, or a Wasm engine.

## Run the comparisons

Build the reference `uv-build` binary from the same pinned revision, then run:

```sh
export TMPDIR="$HOME/code/tmp/uv-build-slim-tests"
mkdir -p "$TMPDIR"
export UV_SLIM_REFERENCE=/path/to/native/uv-build
python3 tests/test_edges.py -v
python3 tests/test_host.py -v
python3 tests/runtime_smoke.py \
  --source /path/to/uv/test/packages/built-by-uv \
  --reference "$UV_SLIM_REFERENCE" --report compatibility.json
python3 tests/test_codegen.py \
  --pywasm /path/to/pinned/pywasm --report codegen-parity.json
python3 tests/streaming_probe.py \
  --reference "$UV_SLIM_REFERENCE" --report streaming-parity.json
python3 tools/isolated_smoke.py \
  --wheel dist/uv_build_slim_wasm-0.1.0-py3-none-any.whl \
  --source /path/to/uv/test/packages/built-by-uv \
  --reference "$UV_SLIM_REFERENCE" --report isolated-parity.json
```

The code-generation comparisons additionally need Node. Runtime checks do not.
To rerun the original Rust suite, use a disposable checkout of the pinned
revision:

```sh
python3 tests/instrument_upstream.py /path/to/disposable/uv
export UV_SLIM_PARITY_SERVER="$PWD/tests/upstream_server.py"
export UV_SLIM_PARITY_REPORT="$TMPDIR/upstream.ndjson"
cargo test --locked --manifest-path /path/to/disposable/uv/Cargo.toml \
  -p uv-build-backend --lib -- --test-threads=1
```

`LICENSES/inventory.json` lists the collected dependency notices. This includes
build dependencies conservatively; Rust standard-library and `pywasm` notices
are included separately. The adjacent direct-AST experiment has its own README
and reports and does not provide the complete backend.
