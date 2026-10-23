# Internal Python bindings

This development package calls the Rust implementations in the current uv checkout. Its API is
unstable and the package is not published. It has a separate Cargo workspace so normal uv builds do
not need Python or PyO3.

From the repository root:

```sh
uv run --project scripts/uv-dev-python --locked validate-python-downloads \
  crates/uv-python-managed/download-metadata.json
```

This checks whether the catalog's architecture, operating system, libc, interpreter variant, and
version can be converted to installation keys. Unknown interpreter names are retained. It does not
fetch artifacts or validate URLs and checksums, and it does not modify the catalog.

The first invocation compiles the extension with the repository's Rust toolchain using the `dev`
profile. Changes to Rust sources invalidate uv's build cache. Run commands through this project to
use the bindings from the current checkout.

The package also exposes `Version`, `VersionSpecifiers`, `Requirement`, and `normalize_package_name`
from uv's packaging crates. For example:

```python
from uv_dev import Requirement, Version, VersionSpecifiers

assert Version("3.15.0rc2") < Version("3.15.0rc10")
assert Version("1.5") in VersionSpecifiers(">=1,<2")
assert Requirement("My_Package[Fast_Mode]>=1").name == "my-package"
```

`VersionSpecifiers` applies Rust's version comparisons without the resolver's prerelease selection
policy. These are small internal wrappers, not a replacement for the full `packaging` API.
`Requirement` exposes normalized names, extras, specifiers, URLs, and markers.

The version-constant maintenance script uses the same `Version` implementation as uv:

```sh
uv run --project scripts/uv-dev-python --locked python scripts/sync-python-version-constants.py
```

To inspect a requirement without running a resolver:

```sh
uv run --project scripts/uv-dev-python --locked uv-debug-requirement \
  "Some_Package[Fast_Mode]>=1; python_version < '3.14'"
```

Pass `--environment environment.json` to evaluate its marker against a complete PEP 508 environment,
and repeat `--extra` for active extras. No environment values are inferred from the machine running
the command.

For interactive investigation, start `uv run --project scripts/uv-dev-python --locked python`:

```python
from packaging.markers import default_environment
from uv_dev import Marker, Requirement

environment = dict(default_environment())
marker = Marker("python_version >= '3.12' and extra == 'fast-mode'")
print(marker.normalized)
print(marker.evaluate(environment, extras=["Fast_Mode"]))
print(Requirement("demo; python_version >= '3.12'").evaluate(environment))
```

`Marker.normalized` is `None` when the expression simplifies to true. `is_true` and `is_false`
report constants recognized by uv's marker simplifier. A `False` result from either property means
the simplifier did not recognize the corresponding constant; it does not establish whether the
expression is satisfiable.

The Hypothesis suite compares selected version and requirement fields, specifier membership, and
marker evaluation with `packaging` using generated inputs. It supplies explicit environments and
enables prereleases for `packaging`'s specifier comparisons. These comparisons exercise the
bindings; resolver and command behavior still belong in uv's Rust and CLI tests.

To run the binding tests:

```sh
uv run --project scripts/uv-dev-python --locked pytest scripts/uv-dev-python/tests
uv run --project scripts/uv-dev-python --locked cargo clippy \
  --manifest-path scripts/uv-dev-python/Cargo.toml --locked -- -D warnings
```
