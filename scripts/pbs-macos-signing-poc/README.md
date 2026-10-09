# PBS macOS signing experiments

These experiments compare three ways of installing and linking against a signed, relocatable CPython
distribution. Each macOS runner creates a disposable self-signed code-signing certificate. No
production signing identity, notarization service, or release publisher is involved.

| Design    | Runtime library                                                              | Consumer linkage                                                                  |
| --------- | ---------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `rewrite` | Its install name changes after signing, followed by explicit ad-hoc signing. | Absolute install name in the modified library.                                    |
| `rpath`   | Its install name becomes `@rpath/libpython3.14.dylib` before signing.        | Each embedding executable supplies its own runtime search path.                   |
| `stub`    | The signed library is unchanged.                                             | An adjacent `.tbd` describes the installed library's absolute path to the linker. |

The inputs are pinned CPython 3.14.7 archives from PBS 20260901. Both ARM64 and x86-64 exercise
native modules, callbacks, a `uv`-created environment with CFFI, a C embedding executable, and a
PyO3 embedding executable. The `rpath` case also requires an embedding executable without a search
path to fail. Immutable designs move the runtime to another prefix, regenerate consumer metadata,
and relink both embedding executables while requiring all native runtime bytes to remain unchanged.

Every design separately passes its signed archive through unmodified `uv 0.12.13` using a local
download manifest. This observation records the installer mutation; the prototype does not claim
that current `uv` already preserves the signatures. The direct runtime experiments model the
proposed linking contract independently of that installer change.

The `.tbd` is a proof of concept generated from the library's exported symbol list. It is not a
production stub generator or a guarantee of compatibility with all linkers, ABI variants, or
embedding tools. C uses `-L`/`-l` and PyO3 receives its library directory through
`PYO3_CONFIG_FILE`; neither gets an extra runtime search path in the stub experiment.

The evidence includes native-file hashes, embedded certificate fingerprints, signature verification
results, dependency load commands, and process output. The private key and keychain are never
uploaded. These experiments establish signature integrity and runtime/linker behavior, not Apple
trust, notarization, Gatekeeper acceptance, or the cause of reported SIGKILL failures.
