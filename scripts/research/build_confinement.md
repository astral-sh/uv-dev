# Build confinement experiment

```sh
python scripts/research/build_confinement.py --uv /path/to/uv --work-dir work
```

This Linux experiment invokes actual `uv pip compile` and `uv build` commands on a bundled,
dependency-free PEP 517 backend. Requirements discovery, metadata preparation, wheel construction,
and their child Python processes probe writes inside/outside the writable workspace, connection to a
host-loopback listener, and visibility of a synthetic environment secret. All writes target
disposable experiment files, and the network probe sends no data. The control must observe all three
capabilities before confinement can claim to block them.

Bubblewrap mounts the host filesystem read-only, grants one writable directory, creates network/PID
namespaces, and supplies an explicit environment. It wraps the whole uv process, **not** only
`PythonRunner`; this measures an external containment option without adding a production sandbox to
uv. Read access to the host filesystem remains available. This is not protection for confidential
files, process-resource exhaustion, kernel exploits, or installed-package execution. Recursive build
dependencies, native toolchains, and real-package compatibility need separate coverage.

JSON reports include every canary, command status, timing, and binary version. Missing bubblewrap or
unavailable namespaces produce a nonzero result; a failed setup must not be counted as blocked
backend actions. `--baseline-only` runs the unconfined control on hosts that cannot create
namespaces. No real malware is used. Both modes execute the bundled backend with the stated
permissions.

[Latch](https://doi.org/10.1145/3488932.3523262) motivates pairing permission policies with live
enforcement and compatibility measurements. [PyFEX](https://arxiv.org/abs/2606.02196) motivates
observing dormant execution paths and descendants. Neither supplies evidence that this small harness
confines every Python build or that a confined package is benign.

The read canary is deliberately outside the writable bind and must remain readable in both modes. A
read-only host bind restricts writes, not disclosure of host files. Descendant observations retain
the parent hook name so all three hook families have explicit child-process coverage.

An entry marker written inside the wrapper distinguishes sandbox setup failure from an entered
command that fails before hooks and a failure after hook execution. Partial hook observations
survive failures. A namespace denial is reported as `setup-failed` and exits unsuccessfully; it is
never interpreted as successful confinement.

`TMPDIR` points inside the writable workspace. The wrapper does not hide host `/tmp`, so placing the
executable or workspace there does not erase its own inputs. The parent of the writable workspace
remains read-only. Failed canary expectations retain all observations with `validation-failed`
before exiting nonzero.
