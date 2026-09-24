# PR1734 installed-index evidence

This archive retains the complete installed-index measurements and standalone analysis for [PR1734],
comparing `36e89bca95a8ddb88440f848c45ecf5085d1f74a` with
`4623a328071784425501a39626beef978a5b1d3f`. The seven files include all 632 ordered timed processes,
316 paired blocks, 72 descriptive screen rows, eight library confidence intervals, eighteen
whole-command confidence intervals, and 36 untimed command qualifications. The ordinary-present
library regression and all inconclusive command controls are retained.

- Archive SHA-256: `ba959aef17b2f77b5bc98c20d52eb835c19e7677af75631db8f81ac3c366ae97`
- External manifest SHA-256: `6f13d8cb660cc556f9b4b4ff4848311517311615720758bbaab0dd19cefa65a6`

Extract into a new directory, then reproduce the complete analysis offline:

```sh
mkdir evidence
tar -xzf installed-index-evidence.tar.gz -C evidence
uv run --no-project --no-config --offline --no-managed-python --no-python-downloads -- python -I -B evidence/analyze.py --bundle evidence --manifest-sha256 6f13d8cb660cc556f9b4b4ff4848311517311615720758bbaab0dd19cefa65a6
```

See `evidence/README.md` for the measured overlays, paired estimator, and warm Linux `profiling`
limits. This reproduces the retained numerical analysis, not the original private machine or a new
timing result. The later lint-corrected benchmark owner was not measured.

[PR1734]: https://github.com/astral-sh/uv-dev/pull/1734
