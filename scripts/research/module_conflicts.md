# Module collision workload

```sh
python scripts/research/module_conflicts.py --baseline /path/to/baseline-uv \
  --candidate /path/to/candidate-uv --work-dir work --check
```

The integration regressions establish warning behavior. This workload expands the comparison to
empty namespace initializers, identical nonempty modules, differences beyond an 8 KiB buffer,
NUL-bearing binary payloads (1 MiB plus one byte by default), and different-length controls. It
builds local wheel archives directly and runs actual installs with the preview detector. The
synthetic extension payloads are never imported. Each sample uses a fresh target and cache, and
checks that the installed shared file contains one complete input payload.

JSON records binary digests/versions, per-sample latency, missed conflicts, and false positives.
`--check` requires correct candidate warnings; it does not assume which detector the baseline
contains. `--large-bytes` and `--repeats` bound the workload. These end-to-end timings include
archive extraction and installation, and do not isolate content-comparison CPU or filesystem reads.
The six synthetic cases cannot establish ecosystem false-positive rates.

[ModuleGuard](https://arxiv.org/abs/2401.02090) motivates examining overlapping module paths. The
workload only covers conflicts within one install command.
