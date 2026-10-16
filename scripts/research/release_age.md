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
must be dotted integers. Extras, markers, direct references, options, and newline-bearing
requirements are rejected before starting the index. This initial representation has one synthetic
universal wheel per release and cannot reproduce platform-specific wheels, yanks, removed artifacts,
index priority, or flat-index behavior. A real study must account for those limitations, missing
observations, advisory-publication lag, and delayed vulnerability fixes. It must not infer that an
older package is safe.

[Hurry Up and Wait](https://doi.org/10.1145/3848003.3848013) motivates measuring this tradeoff. Its
npm timing sample had publication timestamps for only 226 of 520 package/version pairs; its findings
do not establish a PyPI default policy.
