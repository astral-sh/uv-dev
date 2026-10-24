# Cross-build proof of concept

This example builds a CPython extension for macOS x86-64 using an ARM macOS build interpreter. It
uses the experimental PEP 517 protocol described in `docs/reference/cross-builds.md`. Xcode command
line tools provide the cross-compiler. The build never executes the target Python.

Build the frontend and backend from this checkout:

```console
$ cargo build -p uv -p uv-build --locked
```

Download an x86-64 Python SDK into a directory of your choosing:

```console
$ uv python install cpython-3.14.0-macos-x86_64-none --install-dir /path/to/sdk --no-bin
```

Run the example with a native Python 3.12 interpreter and a new work directory:

```console
$ uv run --isolated --no-config --no-project --python 3.12 python -I scripts/cross-build/run.py \
    --target /path/to/sdk/cpython-3.14.0-macos-x86_64-none/lib/python3.14/build-details.json \
    --work-dir /path/to/new/work-directory
```

The driver assembles a local `uv-build` wheel from this checkout's Python shims and debug binary. It
also creates small dependency wheels, then invokes the development `uv build` with build isolation
and `--no-index`:

- `poc-sdk==1.0.0` and dynamically requested `poc-codegen==1.0.0` execute under the build Python.
- `poc-sdk==2.0.0` provides a target header. Its target-only dependency, `poc-runtime`, is also
  requested by the target-requirements hook. Neither target distribution is installed in the build
  environment; both contain traps that would fail if their Python code were executed.
- The example backend resolves the SDK's relative paths against the original `build-details.json`,
  compiles against its Python headers, and reads the additional header from a target wheel.
- `uv-build` prepares metadata and packages the Python files. The example backend adds the native
  extension, writes truthful platform tags, and regenerates `RECORD`.

The result is `project/dist/cross_poc-0.1.0-cp314-cp314-macosx_13_0_x86_64.whl` under the work
directory. `report.json` records the build interpreter, target, target artifacts, and the
extension's architecture as reported by `lipo`.

To also execute the finished extension, append `--verify`. This runs the target Python **after** the
build, requires Rosetta on an ARM Mac, and checks that `cross_poc.answer()` returns `42`. It does
not install Rosetta. A successful run reports fields including:

```json
{
  "build_machine": "arm64",
  "build_sdk_version": "1.0.0",
  "target_python": "3.14",
  "extension_architecture": "x86_64",
  "target_result": 42
}
```

The compiler example supports standard CPython 3.14-or-newer macOS SDKs and a macOS 13 deployment
target. It demonstrates the protocol; it does not make `uv-build` a general-purpose extension
compiler or establish support for other toolchains.
