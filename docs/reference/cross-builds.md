# Explicit build and target inputs for PEP 517

Experimental protocol proposal; not an accepted Python packaging standard.

## Abstract

Extend PEP 517 with a separate role for target build requirements and a way to pass the target
Python installation to wheel-building hooks. The build interpreter runs the backend and executable
tools. Target requirements provide artifacts such as headers and libraries. The frontend resolves
each role for its appropriate environment.

The proposed `target` argument is an absolute path to a `build-details.json` file. Dependency-marker
evaluation, dependency wheel selection, and output acceptance remain frontend responsibilities. The
backend uses the file to configure the Python-specific parts of the build and remains responsible
for producing a correctly tagged wheel.

The additions below are experimental APIs in this branch, not standardized packaging interfaces. The
path-based target argument and separation of dependency roles are the core design. The precise
opt-in convention and target-artifact interface should be confirmed through implementation
experience.

## Relationship to existing standards

PEP 517 makes build requirements available to the interpreter executing the backend. PEP 739 defines
a static description of a Python installation, including its language and implementation versions,
ABI details, and development paths. This proposal uses that existing description as the backend's
target input. [PEP 517](https://peps.python.org/pep-0517/),
[PEP 739](https://peps.python.org/pep-0739/)

The installation description is not a complete description of a dependency-resolution environment. A
frontend may need additional information about the intended deployment environment. Its acquisition
and storage are separate from the hook argument.
[Build-details specification](https://packaging.python.org/en/latest/specifications/build-details/v1.0/)

Here, **build** means the environment executing the backend; **target** means the environment for
which the wheel is produced. The latter corresponds to **host** in GNU terminology. Draft PEP 725
uses `host-requires` for external dependencies and explicitly defers a corresponding PyPI dependency
field in `[build-system]` to possible cross-compilation work. This draft uses `target-requires` to
match the `target` argument; the final spelling should be coordinated with that terminology.
[PEP 725](https://peps.python.org/pep-0725/#adding-a-host-requires-key-under-build-system)

A different target may mean another architecture, operating system, Python implementation, Python
minor version, or ABI. The target interpreter need not be executable on the build machine.

## Hook interface

Keep executable build-requirement discovery unchanged:

```python
def get_requires_for_build_wheel(config_settings=None) -> list[str]: ...
```

Add a separate hook for target requirement additions:

```python
def get_requires_for_target_wheel(config_settings=None) -> list[str]: ...
```

Extend the existing metadata and wheel hooks with keyword-only arguments:

```python
def prepare_metadata_for_build_wheel(
    metadata_directory,
    config_settings=None,
    *,
    target: str | None = None,
    target_wheels: list[str] | None = None,
): ...  # Return the .dist-info directory basename.


def build_wheel(
    wheel_directory,
    config_settings=None,
    metadata_directory=None,
    *,
    target: str | None = None,
    target_wheels: list[str] | None = None,
): ...  # Return the wheel basename.
```

The annotations describe the protocol's types; backends need not use these annotations or require a
Python version that supports their syntax.

The requirements hooks return dependency-specifier strings. They do not receive `target`: a backend
can return marker-bearing requirements, and the frontend evaluates the markers for the role
concerned. Selecting executable tools using target properties that cannot be represented this way
remains an explicit backend-configuration concern or a possible later extension.

The target-requirements hook runs after the executable build requirements have been installed. It
may use those tools or inspect their selected versions. It cannot request further executable
requirements, and it cannot assume that target artifacts have already been obtained. This gives a
bounded sequence without a combined requirements result or a dependency-discovery loop.

The output paths, returned basenames, source working directory, and subprocess conventions retain
their PEP 517 meanings. If the optional metadata hook is absent, the frontend may build the target
wheel, extract its metadata, and retain that wheel as the result.

## Meaning of `target`

`target` is either:

- `None`: build for the interpreter executing the backend, using its native Python configuration.
- An absolute filesystem path, represented as `str`, to a valid `build-details.json` describing the
  intended target Python installation.

For example:

```python
build_wheel(
    wheel_directory="/work/dist",
    target="/sdk/cpython-aarch64/build-details.json",
    target_wheels=[
        "/work/target-dependencies/numpy-2.3.0-cp314-cp314-manylinux_2_28_aarch64.whl"
    ],
    config_settings=backend_toolchain_settings,
)
```

The filename in this example illustrates the interface, not the availability of a particular
published artifact. `backend_toolchain_settings` denotes the settings appropriate to the selected
backend.

Implementations of this extension must support build-details schema `1.0`. A backend rejects
unsupported schema versions and missing data required for the requested build. The file supplies
Python installation facts, such as version, implementation, extension suffixes, ABI flags, header
paths, and Python library information; fields are consumed according to the build-details
specification and their applicability to that Python installation.

There is no new wrapper JSON schema, marker dictionary, compatible-tag list, or output-policy field
in the hook argument.

The file and SDK must be accessible on the build machine. Resolve relative paths according to
build-details: `base_prefix` may be relative to the file's directory, while development paths may be
relative to `base_prefix`. An SDK provider or frontend relocating the description must preserve
those meanings. Copying the JSON file to another directory without adjusting relative paths can
change what it describes.

A target path such as `/usr/include/python3.14` must not accidentally refer to the build machine's
headers. The provider must expose a description whose development paths locate the actual target
SDK. Merely knowing the intended architecture is insufficient.

The frontend keeps the supplied file and SDK stable from target validation and dependency resolution
through metadata preparation and wheel creation. The backend treats them as read-only. The target
interpreter path, if present in the description, is not a request to execute it. A provider may
generate a conforming file for an older Python installation; the target need not be Python 3.14 or
newer.

## Frontend and backend responsibilities

| Concern                                                                    | Owner                                     |
| -------------------------------------------------------------------------- | ----------------------------------------- |
| Target dependency-marker values                                            | Frontend                                  |
| Resolving build requirements for the executing interpreter                 | Frontend                                  |
| Resolving target requirements and selecting compatible wheels              | Frontend                                  |
| Checking output compatibility and requested portability policy             | Frontend                                  |
| Reading target Python ABI and SDK details                                  | Backend                                   |
| Configuring compilation and selecting target inputs                        | Backend, using its existing configuration |
| Producing a wheel whose metadata and tags truthfully describe its contents | Backend                                   |

The frontend may derive some resolution information from build-details, but must not assume that
file determines every marker or compatibility tag. For example, a platform string and Python ABI do
not by themselves establish a deployment's libc baseline or all kernel-related marker values.
Additional information can come from the frontend's target selection, SDK provider, or explicit
configuration. Missing target information must not silently default to the build machine.

The separately maintained resolution environment must agree with the target installation wherever
their facts overlap. The frontend must reject known inconsistencies in Python version and
implementation, and in platform or ABI information where those can be checked. Resolving
dependencies for one Python configuration while supplying an SDK for another is invalid even if a
broadly compatible output tag would pass the final check. This consistency requirement does not
imply that build-details supplies every deployment property.

The frontend checks `Requires-Python`, evaluates markers, and chooses dependency wheels against the
environment associated with each requirement role. Runtime `Requires-Dist` markers remain ordinary
metadata expressions; they must not be evaluated for the build machine and discarded during metadata
preparation.
[Dependency specifiers](https://packaging.python.org/en/latest/specifications/dependency-specifiers/)

Frontend compatibility checking does not transfer wheel tagging to the frontend. The backend still
computes tags describing what it actually built. It may need compiler, sysroot, deployment-baseline,
or portability settings that are not Python installation facts. Those remain backend/toolchain
configuration, for example through `config_settings` and existing toolchain interfaces. A frontend
requesting a particular output policy must configure the build accordingly or reject a result that
does not meet it. It must not make an incompatible artifact appear compatible by renaming it.

## Static and dynamic target requirements

A proposed static declaration is:

```toml
[build-system]
requires = ["example-backend"]
build-backend = "example_backend"
target-requires = ["numpy>=2,<3"]
```

`target-requires` is an optional list of dependency-specifier strings, defaulting to `[]`. It
declares target artifacts needed to build wheels; it does not declare the produced wheel's runtime
dependencies. It does not change sdist requirements.

The target-requirements hook returns additions to that static list. The ordinary build-requirements
hook retains its existing role:

| Requirement origin               | Resolution and consumption                                   |
| -------------------------------- | ------------------------------------------------------------ |
| `[build-system].requires`        | Build interpreter; importable bootstrap requirements.        |
| `get_requires_for_build_wheel`   | Build interpreter; additional executable build requirements. |
| `[build-system].target-requires` | Target; artifact inputs.                                     |
| `get_requires_for_target_wheel`  | Target; additional artifact inputs.                          |
| Wheel `Requires-Dist`            | Destination environment; runtime dependencies.               |

For example, the target hook can return:

```python
return [
    "dependency<2; python_version < '3.12'",
    "dependency>=2; python_version >= '3.12'",
]
```

The frontend evaluates those markers against the target. Requirements returned by the ordinary build
hook continue to use build-environment markers. No requirement's role is inferred from its package
name.

A distribution can appear in both roles. Identical version ranges do not guarantee equal versions.
If the backend needs matching versions, the staged discovery order lets it inspect a resolved build
package and request that exact version for the target. That can fail if no target artifact exists;
the protocol does not require resolver backtracking across roles. It also does not assume that a
native package's configuration API returns suitable target headers.

The exact bootstrap artifacts must remain fixed while completing a build sequence, so installing
additional executable requirements cannot replace the backend that performed discovery. Conflicts
require an error or a fresh sequence with a different bootstrap resolution. After executable
requirements are finalized, they stay fixed through target discovery, metadata, and build.

## Explicit opt-in and native compatibility

A proposed opt-in convention is that the backend object exposes a callable
`get_requires_for_target_wheel`. Implementing this hook promises support for the entire protocol,
including the new keyword arguments on the wheel hook and on the metadata hook if implemented. A
backend with no dynamic target requirements returns `[]`.

A frontend detects this capability on the backend object resolved using PEP 517's normal
`build-backend` and `backend-path` rules. It must not infer support merely from acceptance of
arbitrary keyword arguments, and must not catch a general `TypeError` and retry as a native build.
An unsupported explicit target is an error.

New frontends can use the protocol for native builds too: `target=None` selects native Python
configuration, while a list in `target_wheels` supplies artifact requirements. The roles remain
distinct even when both environments match.

Legacy callers omit both new arguments, leaving `target_wheels=None`. The backend's ordinary native
path remains available where supported. In the new protocol, the frontend always passes a list,
including `[]` when empty. A non-`None` `target` requires a list in `target_wheels`.

Existing projects and legacy hook signatures need no change. Projects adopting `target-requires`
cannot assume older frontends will recognize or provision it. Their backend must provide a
self-contained legacy native path, or fail clearly when required artifact inputs are absent; older
frontend acceptance of the new table key cannot be guaranteed. The extension must not claim
universal old-frontend compatibility for newly declared target requirements.

A dedicated capability hook is an alternative to using target-requirement discovery as the opt-in
signal. Either convention must promise coherent support across metadata and wheel building;
finalizing that spelling does not require making `target` a structured context object.

## Target artifacts

`target_wheels` contains absolute paths to immutable wheel archives selected for the target. The
frontend does not install them into the build interpreter, append them to its import path, process
their `.pth` files, or expose their entry points on the build `PATH`. The backend reads metadata and
extracts build inputs into its own staging area. A Python API or command that must execute belongs
in the build role.

The initial design supplies the resolved target dependency closure, with one selected distribution
per normalized project name. Ordering is not significant. This uses familiar dependency-resolution
semantics, but may obtain runtime artifacts a header-only consumer does not need. An unavailable
runtime-only dependency can therefore block the build. Whether to provide an artifact-only mode is
an implementation question, with consequences for transitive constraints and input discovery.

Wheel archives avoid specifying another virtual environment or a universal sysroot layout. Consuming
package-specific headers and libraries still requires backend knowledge. The artifact interface
should be prototyped with NumPy and at least one other provider before adoption.
[Wheel format](https://packaging.python.org/en/latest/specifications/binary-distribution-format/)

When a target dependency has no compatible wheel, a frontend may recursively build its source for
the same target Python and resolution environment, or report an unavailable artifact. Each
dependency build has its own backend configuration and output acceptance policy; the requested
project's restrictions are not blindly inherited. A dependency needed to execute on the build
machine is built for the build environment instead.

Ordinary runtime cycles can be resolved when their artifacts are available. If obtaining a required
artifact would depend on finishing an active build that already requires it, the frontend reports
the artifact-production cycle. Merely resolving cyclic metadata does not produce the missing wheel.

## Execution sequence

1. Select the target Python SDK, frontend resolution environment, and backend toolchain settings. A
   native target-aware build may use `target=None`.
2. Resolve and install bootstrap requirements for the build interpreter, then establish backend
   opt-in.
3. Call `get_requires_for_build_wheel`, if implemented, and install its additions while retaining
   the selected bootstrap artifacts.
4. Call `get_requires_for_target_wheel`. Resolve its additions together with `target-requires` for
   the target and obtain their wheel artifacts.
5. Call the metadata hook with `target` and `target_wheels`, if needed and implemented. Otherwise
   build a wheel with the same inputs, obtain its metadata, and retain it.
6. Build the wheel if necessary, passing the same target file, configuration, dependency artifacts,
   and prepared metadata directory.
7. Check the returned wheel's identity, metadata, `Requires-Python`, tags, and frontend output
   policy.

Hooks run under the actual build interpreter. The frontend does not make `sys`, `sysconfig`, or
`platform` impersonate the target. Native tools remain executable, and target data remains
separately available.

## Metadata, caches, and limitations

The metadata hook returns a `.dist-info` basename within its input directory. When relying on
prepared metadata, the frontend passes that directory to `build_wheel` unchanged, including
unrecognized auxiliary files. The backend produces a wheel with identical metadata; auxiliary
preparation files do not all have to be copied into the wheel. The target file, SDK, configuration,
and resolved artifacts must remain consistent across those calls.

Cache identity must distinguish the source, backend and executable requirements, build interpreter,
target description contents, resolved SDK inputs, target artifacts, and relevant toolchain
configuration. A filename alone is not an adequate target identity. If changes to external SDK or
toolchain inputs cannot be detected, avoid reusing results that depend on them. The protocol does
not promise hermetic builds.

The frontend's tag checks catch incompatible declarations, not incorrectly compiled binaries bearing
plausible tags. Correct ABI, linkage, and portability remain backend responsibilities. Pure-Python
and stable-ABI wheels are permitted when their contents and metadata justify their compatibility.

This extension covers wheel building and metadata. It does not provision compilers, resolve
non-Python dependencies, standardize target execution or emulation, extend editable installs, or
orchestrate multiple SDKs into a universal binary. Existing sdist hooks retain their ordinary
behavior.

## Prototype in uv and uv-build

Enable the experimental frontend with `--preview-features cross-builds`:

```console
$ uv build --wheel --preview-features cross-builds \
    --python 3.12 \
    --target /sdk/lib/python3.14/build-details.json \
    --python-platform aarch64-unknown-linux-gnu
```

`--python` selects the interpreter that executes build tools. `--target` selects the Python SDK.
`--python-platform` supplies the frontend's platform and deployment policy. It must agree with the
SDK's operating system and architecture. Python versions and ABI flags come from the description;
the frontend never queries the target interpreter. Runtime `Requires-Python` constrains the target,
not the build interpreter.

The prototype implements static and dynamic target requirements in `uv build`, including their
transitive dependencies. It uses the existing indexes and `--find-links` configuration, resolves
with target markers and tags, and hands wheel archives to opted-in backends. Build constraints apply
only to executable dependencies. Target requirements do not yet support required hashes; requests to
enforce them fail instead of weakening that policy.

uv's cache stores extracted wheels. For this prototype, uv repackages cached contents, including the
cached `RECORD`, into private archives for the hook sequence. Extraction can repair `RECORD`, so
even that file can differ from the downloaded artifact. ZIP bytes and archive hashes can also
differ. The frontend neither installs these contents nor exposes their entry points to the build
interpreter.

The frontend calls the metadata hook when available, passes its directory unchanged to the wheel
hook, and checks the resulting core metadata, `Requires-Python`, filename tags, and `WHEEL` tags. It
detects changes to the target description between hooks. A rejected output is discarded before being
published to the output directory. `uv build` does not reuse a cached output wheel; the caller must
keep the SDK and toolchain unchanged during the build.

`uv-build` implements the target-requirements hook, accepts the new keyword arguments, and rejects
unsupported target-description schema versions. It continues to produce platform-independent pure
Python wheels. Its bundled fast path validates the target output too. Projects declaring static
target requirements use PEP 517; with `cross-builds` enabled, those requirements are resolved.
`scripts/cross-build/README.md` describes a compiler backend layered on `uv-build` that builds a
real extension using a foreign CPython SDK and target headers.

Current implementation boundaries:

- Explicit targets require `--wheel` and a CPython 3.8-or-newer description for Linux, macOS, or
  Windows. The macOS compiler example has narrower requirements documented alongside it.
- Target source distributions are rejected. Executable source dependencies use ordinary native
  builds. Cross-builds during installation, locking, or editable builds are not implemented.
- Without an explicit target, the preview feature uses the protocol for opted-in PEP 517 wheel
  builds with `target=None`. Legacy backends receive their original hook arguments.
- Platform markers use uv's existing `--python-platform` semantics, including its approximations for
  platform release and version. This is not a complete deployment-environment description.
- Bootstrap installation retains uv's existing behavior. Freezing the exact bootstrap artifacts
  across additional executable-requirement resolution, as required by the proposed protocol, is
  still needed before treating this as a complete implementation. Capability discovery also happens
  after executable requirements are installed, rather than immediately after bootstrap.

## Further acceptance criteria

Before standardization, exercise the transport in `pyproject-hooks` and additional backends,
including a Cython extension using target NumPy headers and a Rust extension using a target Python
SDK. Confirm free-threaded and debug ABI behavior, SDK relocation, explicit output policies,
recursive target builds, and the artifact interface with real header and library consumers.

The final design should settle opt-in discovery, static-field naming and migration, and artifact
consumption without expanding the `target` argument beyond the Python installation description
unless concrete backend requirements justify it.
