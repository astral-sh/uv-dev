# PR1840 pip-show reverse-dependency index

This archive retains the complete `uv pip show` measurements and standalone analysis for [PR1840],
comparing `6792c507b5b0760f7d07d9de55b0c67492b7ce46` with
`ceaa7293604bc26fa364d5fdf309de0de3e4f82a`. The seven files include all 216 ordered wall-clock
observations, 108 balanced process pairs, and nine pointwise paired 95% intervals, including the
adverse and inconclusive endpoints. The study distinguishes the real Jupyter environment from the
controlled 512- and 6,400-package metadata graphs.

- Archive SHA-256: `32efe963994d039f0adc225a32a9f6e2dc069eb5a3e635f3ad4896aea8d1f0bd`
- External manifest SHA-256: `5cc4f3bb5405a0098d0f5e6641b25db911330c325dfb516d82ac52fa2881e268`

Extract into a new directory, then reproduce the complete analysis offline:

```sh
mkdir evidence
tar -xzf pip-show-evidence.tar.gz -C evidence
uv run --no-project --no-config --offline --no-managed-python --no-python-downloads -- python -I -B evidence/analyze.py --bundle evidence --manifest-sha256 5cc4f3bb5405a0098d0f5e6641b25db911330c325dfb516d82ac52fa2881e268
```

See `evidence/README.md` for the clock, pairing, estimator, profile, and workload limits. This
reproduces the retained numerical analysis, not the original private machine or a new timing result.

[PR1840]: https://github.com/astral-sh/uv-dev/pull/1840
