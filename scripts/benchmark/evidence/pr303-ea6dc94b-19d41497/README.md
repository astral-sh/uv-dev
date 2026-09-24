# PR303 legacy-egg namespace filtering

This archive contains the complete-operation measurements and standalone analysis for [PR303]. The
measured `ea6dc94b6f23` to `19d414971b5b` pair uses an equal benchmark overlay and the same
production change as the proposed `94ac9f5f3369` to `a4b71961e502` review pair; it is not a literal
build of that review pair. The seven files retain all 720 ordered samples, the 3,240 command proofs,
and all 20 pointwise paired 95% intervals, including adverse, inconclusive, and bypass results.

- Archive SHA-256: `b7e2a0b9a1d957f2bfd5989efe4ffa4942beedc63aba877eb57423df1267279c`
- External manifest SHA-256: `87a476f1a39c4ca73981face779c29e3cfe6cd0ae691f36e743763997d4ca42b`

Extract into a new directory, then reproduce the complete analysis offline:

```sh
mkdir evidence
tar -xzf namespace-evidence.tar.gz -C evidence
uv run --no-project --no-config --offline --no-managed-python --no-python-downloads -- python -I -B evidence/analyze.py --bundle evidence --manifest-sha256 87a476f1a39c4ca73981face779c29e3cfe6cd0ae691f36e743763997d4ca42b
```

See `evidence/README.md` for the clock, pairing, estimator, profile provenance, and workload limits.
The digest-only command proofs do not independently reproduce private process or filesystem
qualification.

[PR303]: https://github.com/astral-sh/uv-dev/pull/303
