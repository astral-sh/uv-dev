# Dependency-group Python marker measurements

Matched component measurements for [uv-dev#244](https://github.com/astral-sh/uv-dev/pull/244),
comparing `c8d39cb1f6fca5a00a5d727d2181a479a13f5d29` with
`a58f5c416fc5049577acc68b694f75fc0569e65d`. The two isolated Rust `1.97.1` release builds use the
same lockfile, allocator, feature selection, and study overlay. Their normalized source trees
differ only in `crates/uv-workspace/src/dependency_groups.rs`.

The change improves three selected multi-requirement controls, but makes a configured empty group
about 10.48 times slower. The authentic uv manifest and five other cases were too noisy to admit an
A/B comparison. These observations do not establish an effect for a later fix.

| Case | A/B pairs | Head/base [pointwise 95% interval] | Outcome |
| --- | ---: | --- | --- |
| `real/uv` | — | — | A/A-inconclusive |
| `control/unconfigured-empty` | 24 | 1.012112 [0.985185, 1.039775] | Within ±5% |
| `control/unconfigured-32` | 12 | 1.018574 [0.959172, 1.081654] | Inconclusive |
| `control/configured-empty` | 12 | 10.479489 [10.368837, 10.591321] | At least 5% slower |
| `control/simple-singleton` | — | — | A/A-inconclusive |
| `control/exclusions-singleton` | — | — | A/A-inconclusive |
| `control/simple-32` | 12 | 0.584685 [0.535612, 0.638254] | At least 5% faster |
| `control/exclusions-32` | 12 | 0.345408 [0.336258, 0.354807] | At least 5% faster |
| `control/exclusions-2048` | — | — | A/A-inconclusive |
| `control/included-overlap` | — | — | A/A-inconclusive |
| `control/duplicate-includes` | 12 | 0.715561 [0.677541, 0.755713] | At least 5% faster |
| `control/included-empty-adverse` | — | — | A/A-inconclusive |

The clock covers `FlatDependencyGroups::from_pyproject_toml` on a preparsed `PyProjectToml`,
including group collection and flattening, requirement parsing, marker construction and
composition, and result allocation and destruction. TOML parsing and the complete behavior oracle
are outside timing. The oracle may warm process-global marker interning. There is no cold-process
or whole-command claim.

Each contrast has twelve fresh-process A/A pairs in alternating order. A fixed noise rule selects
twelve or twenty-four A/B pairs, or declares A/A inconclusive. The estimator computes a Student-t
interval on the mean paired log ratio and exponentiates its center and endpoints. Intervals are
pointwise, not simultaneous. All prespecified adverse and inconclusive outcomes are retained.

## Reproduce the report

Run `python3 analyze.py` in this directory. It authenticates `manifest.json` and `data.tar.gz`,
replays the complete calibration and process order, and runs the unchanged recorded estimator on
the original A/A and A/B bytes. The resulting fixed design and report must match the recorded bytes
exactly. No uv executable, package index, network access, or third-party Python package is needed.

The archive contains all calibration attempts, ordered A/A pairs, the sealed design, ordered A/B
pairs, and the complete report. Native process receipts are represented by their original hashes
and relative references; execution-host logs are retained separately. The manifest identifies the
original plan, fixture, normalized source trees, build receipts, and executable bytes.

## Repeat the experiment

Apply `overlays/base.patch` or `overlays/head.patch` to a clean checkout of its matching immutable
commit. The patches contain the exact measured study example, common benchmark target, and the
single `serde_json` dependency edge added to `uv-bench` in the lockfile. Build each arm in a different
empty target and build directory:

```sh
cargo +1.97.1 --config 'build.build-dir="<absolute-arm-target>"' build \
  --frozen --release --jobs 1 --target x86_64-unknown-linux-gnu \
  --target-dir <absolute-arm-target> -p uv-bench \
  --example dependency_groups_study --message-format=json-render-diagnostics
```

Extract `fixture-cases.json` from `data.tar.gz`. Its twenty-three cases, expected outputs, ordering,
and per-case hashes are unchanged; only `authentic_manifest.path` is made repository-relative.
This gives the portable fixture a different whole-file digest. A new measurement must record that
new digest and must not be labeled as a replay of the historical sample bytes.

The executable accepts `admit <fixture>` and
`sample <fixture> <case> flatten_groups <iterations>`. Use the calibration and sampling rules in
the archived `plan.json`, starting with a fresh calibration and A/A pass on a quiet machine. The
separate end-to-end owners are [uv-dev#1572](https://github.com/astral-sh/uv-dev/pull/1572) and
[uv-dev#1578](https://github.com/astral-sh/uv-dev/pull/1578).
