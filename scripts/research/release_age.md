# Release-age replay

```sh
python scripts/research/release_age.py --uv /path/to/uv --work-dir work --check
```

The script serves a frozen corpus through a loopback Simple JSON API, including per-file upload
times and hashed PEP 658 metadata. Actual `uv pip compile` resolutions compare no age policy with
1-, 2-, 7-, and 14-day cutoffs calculated from the corpus timestamp. It creates pure-Python fixture
wheels but executes no package code. Registry traffic stays on loopback, and each resolution has a
fresh cache. `--days` selects another policy set; its first entry must be `0` (baseline).

The checked-in synthetic corpus shows a transitive release excluded by a short delay, an older
vulnerable fixture selected when a fix is also too recent, unsatisfiable requirements with only
recent releases, and missing upload times. These labels are invented fixture roles and are **not
empirical security data**. `--check` compares the selected package/version mappings with the
fixture's expectations. JSON output records the corpus digest, uv/Python versions, cutoffs, selected
versions and labels, root/transitive scope, failures, and elapsed time.

For historical research, supply `--corpus` with the same schema and provenance for
timestamps/labels. `expected` is needed only with `--check`. Requirements must use normalized names
and optional comma-separated numeric version bounds, such as `age-library>=1,<3`. Release versions
must be dotted integers. `~=` requires at least two numeric components, such as `~=1.0`, and `.*` is
allowed only with `==` or `!=`. Equivalent release declarations such as `1` and `1.0` are rejected;
list their artifacts under one release instead. Extras, markers, direct references, options, and
newline-bearing requirements are rejected before starting the index. Each release may supply
`files`, with a wheel `tag` and optional `upload_time` for each file; without it, a single universal
wheel uses the release upload time. The synthetic corpus does not reproduce real wheel contents,
yanks, removed artifacts, index priority, or flat-index behavior. A real study must account for
those limitations, missing observations, advisory-publication lag, and delayed vulnerability fixes.
It must not infer that an older package is safe.

[Hurry Up and Wait](https://doi.org/10.1145/3848003.3848013) motivates measuring this tradeoff. Its
npm timing sample had publication timestamps for only 226 of 520 package/version pairs; its findings
do not establish a PyPI default policy.

Two file-level scenarios distinguish a release's earliest upload from eligible artifacts.
`age-files` has an old Python 2 wheel and a recent Python 3 wheel for version 2.0, so Python 3 falls
back to version 1.0 under an age cutoff. `age-split` instead has an older wheel compatible with both
Python 2 and 3, so version 2.0 remains available. JSON output includes filenames retained in the
generated `pylock.toml` and separate release/file counts. These are candidate artifacts in the lock
output, not a claim that each was installed. `versions_changed_from_baseline` compares `selected`
with the no-cutoff baseline, including a change to or from `null` when resolution has no solution;
compare `selected_files` for changes to candidate artifacts.
