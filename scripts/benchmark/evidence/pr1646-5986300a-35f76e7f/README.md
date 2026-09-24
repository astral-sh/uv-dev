# PR1646 wheel-install replay

This archive retains the original Criterion measurements and standalone analysis for the
exact-parent/head replay in [PR1646]. It contains all 709 reviewed files, including the complete raw
data, source and fixture identities, and paired analysis. The historical CodSpeed attribution
remains unresolved.

- Archive SHA-256: `7a0af5b5ad17f0954c57d9cf28a0b66e6b96bf72a9193ff6802fd00199d91c12`
- External manifest SHA-256: `27ada03f2830cf0058433bc33315e133a549afe16575b680891f779b0442e237`

Extract into a new directory, then reproduce the complete analysis offline:

```sh
mkdir evidence
tar -xzf criterion-evidence.tar.gz -C evidence
uv run --no-project --no-config --offline --no-managed-python --no-python-downloads -- python -I -B evidence/analyze.py --bundle evidence --manifest-sha256 27ada03f2830cf0058433bc33315e133a549afe16575b680891f779b0442e237
```

See `evidence/README.md` for the clock, pairing, confidence interval, and scope.

[PR1646]: https://github.com/astral-sh/uv-dev/pull/1646
