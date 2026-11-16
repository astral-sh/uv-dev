# uv pip compile --no-deps does not warn about invalid extras

Issue: astral-sh/uv#23268

Classification: enhancement

## Summary

The reporter requests validation of extra names during `uv pip compile --no-deps`, while keeping only top-level packages in the compiled output. Their example requests `pandas[exce]`: normal compilation emits the missing-extra warning, while direct-only compilation succeeds without it. They use `--no-strip-extras` to retain the requested extra in the output. The report specifies uv 0.13.8, Windows 11, and Python 3.13.17.

The missing warning is an explicit metadata-fetching tradeoff introduced by astral-sh/uv#7127 in September 2024. That PR removed precisely this warning from an integration snapshot. The current checkout retains the same expectation and implementation. No existing issue or pull request was found requesting validation specifically with `--no-deps`.

## Draft response

This behavior was introduced deliberately in astral-sh/uv#7127: with --no-deps, uv skips fetching distribution metadata, including the declared extras, to avoid additional downloads that can include entire wheels. The existing compile test explicitly expects no warning in this mode.

Validating extras while keeping only top-level packages would require revisiting that tradeoff. For now, you can run a separate compile without --no-deps to obtain the warnings, while retaining --no-deps for your top-level-only output. astral-sh/uv#2828 separately discusses treating invalid extras as errors.

## Classification

The requested capability is extra-name validation during direct-only compilation. astral-sh/uv#7127 explicitly chose to skip the metadata required for that check and changed the exact integration test to require no warning; the current source still implements that policy. The report establishes no incorrect dependency selection or failure of the top-level-only compilation contract. Restoring this check would revisit an explicit performance tradeoff. The earlier astral-sh/uv#1801 panic fixed by astral-sh/uv#1812 has not returned, and astral-sh/uv#2828 concerns errors rather than this missing validation mode.

The initial issue label is bug; enhancement is the recommended classification. This recommendation rests on the command's direct-only dependency semantics and the explicit choice to omit this additional check, rather than merely on the existence of a limitation. The report requests checking distribution metadata in a mode designed to avoid fetching it; it does not establish incorrect top-level pins or missing dependencies that this mode promises to resolve.

Earlier versions did validate these extras. This is not evidence of a new regression in uv 0.13.8: the later removal was deliberate, and the historical crash itself is absent from this report. No specific release introducing an unintended regression has been established.

## Related

- astral-sh/uv#7127 (pull request, merged) — Skip metadata fetch for `--no-deps` and `pip sync`. Directly establishes the reported behavior: this change intentionally stopped fetching distribution metadata with --no-deps, explicitly accepting that extras could no longer be validated to avoid potentially downloading wheels twice. Its diff removes the invalid-extra warning from the no_deps_invalid_extra snapshot.
- astral-sh/uv#2828 (issue, open) — (🎁) Fail and show an error when an invalid extra is requested. Adjacent validation-policy discussion. Maintainer comments confirm that pip compile normally warns and keep this issue open to decide whether invalid extras should cause failure. It does not track restoring warnings while skipping dependency resolution, so it is not a duplicate.
- astral-sh/uv#1812 (pull request, merged) — Wait for distribution metadata with `--no-deps`. Historical implementation of the requested diagnostic: its no_deps_invalid_extra test expected a warning. It fixed the panic reported in astral-sh/uv#1801; astral-sh/uv#7127 later explicitly removed the warning as a performance tradeoff. The current report does not reproduce that historical panic.

## Supporting evidence

- `crates/uv/tests/pip_compile/pip_compile.rs:8513`: the comment on `no_deps_invalid_extra` explicitly says “We don't warn here.” The test compiles `flask[empty]` with `--no-deps`, exits successfully, and snapshots no warning. This already covers the central reported behavior with a different package.
- `crates/uv/tests/pip_compile/pip_compile.rs:4606`: `missing_registry_extra` verifies the warning during ordinary compilation, matching the reporter's comparison.
- `crates/uv-resolver/src/resolver/mod.rs:1887`: candidate metadata requests require transitive dependency mode. At line 2023, direct-only dependency lookup returns before waiting for metadata.
- `crates/uv-resolver/src/resolver/resolution.rs:45`: registry metadata is optional, with a comment explaining that direct-only resolution need not fetch it.
- `crates/uv-resolver/src/resolution/output.rs:318`: `MissingExtra` diagnostics are created only when distribution metadata is available and the requested name is absent from `provides_extra`.
- `crates/uv-pip-commands/src/compile.rs:806`: compilation still prints the diagnostics that resolution supplies. The source supports missing validation metadata as the mechanism for the registry example, rather than a general suppression of warning output.
- astral-sh/uv#7127, merged September 7, 2024: the description explicitly accepts losing extra validation to skip metadata fetching. Its diff removes the warning and adds the current test comment. No PR discussion or review comments provide a later reversal.
- astral-sh/uv#1812, merged February 21, 2024: its original test expected this warning; the PR fixed the metadata-related panic in astral-sh/uv#1801. astral-sh/uv#5392 and astral-sh/uv#5918 also describe the older metadata-fetching policy, before astral-sh/uv#7127.
- astral-sh/uv#2828: a maintainer confirms ordinary compile warnings and explicitly keeps the issue open for the separate decision about errors. Its linked astral-sh/uv#2829 added diagnostic printing to `pip install`, not validation under `--no-deps`.
- The performance rationale in astral-sh/uv#7127 links to a maintainer comment on astral-sh/uv#5073 explaining that registries without metadata endpoints or range requests require downloading wheels to inspect requirements.

The current source establishes the general registry path; no Windows-specific condition appears in the relevant checks. Retaining extra names in output does not itself fetch the metadata required to validate those names.

## Search scope and candidates ruled out

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests using literal --no-deps/extra combinations, the exact 'does not have an extra' diagnostic, 'invalid extras', pandas[exce], and version 0.13.8; conceptual searches covered unknown/nonexistent/missing extras, validation, warnings, diagnostics, and pip compile. Separate metadata-oriented and historical-fix searches used PR text, commit search, and issue timelines. Inspected candidate bodies and comments, relevant PR diffs, and the current resolver and integration tests. Direct PR text searches returned no matches; reference chains and commit search identified the historical PRs. Ruled out astral-sh/uv#1801 and astral-sh/uv#3184 as crash reports, astral-sh/uv#7211 as a warning caused by a transitive requirement, and astral-sh/uv#13963 as installed-environment validation. No existing discussion was found requesting this specific change.

The report was decomposed before searching into the missing warning, requested validation independent of dependency traversal, the `pip compile` resolver/diagnostic path, the `--no-deps` trigger, retention through `--no-strip-extras`, and the exact diagnostic and package-extra spelling. Platform and version were treated as context rather than assumed causes.

The strongest excluded issue, astral-sh/uv#1801, used the same command and extras trigger but reported a panic; its fixing PR is retained above for its exact warning test. astral-sh/uv#3184 and its fix astral-sh/uv#3188 concern an unregistered metadata task for editable installs. astral-sh/uv#7211 concerns an emitted warning originating in a transitive requirement. astral-sh/uv#13963 requests validation of installed environments through `pip check`. None centralizes the present request.

## Verification and limits

Evidence comes from the issue, repository history accessed through authenticated read-only GitHub CLI requests, PR diffs, and current source and snapshots. No commands from issue content were executed. No build or runtime reproduction was performed, so the reporter's exact package versions and Windows output were not independently reproduced.

Only this handoff README was written. Checkout files and GitHub were left unchanged.
