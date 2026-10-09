# Darwin release markers lose opaque-string semantics during evaluation and simplification

Issue: astral-sh/uv#22259

Classification: bug

Status checked: October 9, 2026. The issue is open. The four related open pull requests are drafts; none is a merged repair. These GitHub statuses come from the existing context review and were not refreshed during reproduction.

## Summary

Both reported public `uv-pep508` API behaviors were independently reproduced against checkout `01b62808962d7abfe2d10f43d652f357d8038202` using Rust 1.99.0 on Linux x86_64, with Darwin explicitly selected in the marker environment:

- For `platform_release = "not-a-version"`, `platform_release == '24'` and `marker.negate()` both evaluate false.
- For `platform_release = "3-invalid"`, `platform_release >= '9'` and `platform_release < '24'` each evaluate false, but their parsed disjunction evaluates true. Its printed form includes an unconditional `sys_platform == 'darwin'` branch.

These observations establish logical inconsistencies independently of the broader opaque-string ordering policy. The reporter uses synthetic API inputs and does not claim failure on ordinary macOS releases or demonstrate an end-to-end installation failure. This reproduction likewise does not establish ordinary-host installation impact.

The report used source base `eaa0fb829a581ef50fd75c53b5ca964bc73d31c6`, reproduction-only commit `946e286a63dc6972e45bc2f43a80301439f2d7b7`, and Rust 1.99.0 on Darwin arm64. The independent run used the current checkout as a read-only library dependency, without checking out or executing the draft branch. Python does not participate in marker evaluation.

astral-sh/uv#22258 supplies the original reproductions. astral-sh/uv#22272 and astral-sh/uv#22295 propose different fallback policies and remain unmerged drafts. astral-sh/uv#21766 introduced the affected path; astral-sh/uv#22304 concerns explicit macOS targets.

## Classification

Bug: opaque strings are accepted by `MarkerEnvironment`, yet the observed results violate logical negation and disjunction consistency. The second case does not require choosing between lexical fallback and the current specification's opaque strict-ordering rules: both individual predicates actually evaluate false in this implementation, while their disjunction evaluates true.

The broader fallback policy remains undecided. The companion reproduction and subsequent proposed fixes respond to this issue, so they do not make it a duplicate. No prior repair of these concrete failures was established. astral-sh/uv#19808 deliberately excluded Version | String fields from its string-ordering change.

The issue was opened on October 6, 2026. astral-sh/uv#22258 is its companion reproduction, created minutes earlier; its body explicitly identifies this issue and says it contains intentionally failing tests. astral-sh/uv#22272 and astral-sh/uv#22295 were opened on October 7 and explicitly propose closing this issue. These are follow-up proposals, not independent canonical discussions. The introducing change, astral-sh/uv#21766, merged September 17, 2026. The evidence does not establish recurrence of a previously fixed bug.

## Reproduction

Outcome: **reproducible**. Two independently reconstructed public-API tests failed at the same assertions as the report; Cargo exited 101 with zero passed and two failed.

### Environment and isolation

- Host: Linux x86_64. The supplied environment uses `sys_platform = "darwin"`, `platform_system = "Darwin"`, and `platform_machine = "arm64"`; no macOS host is required for these API calls.
- Tested library: `uv-pep508 0.0.92` from checkout `01b62808962d7abfe2d10f43d652f357d8038202` (workspace uv version 0.13.0).
- Compiler: `rustc 1.99.0 (b940084d7 2026-09-28)`; Cargo `1.99.0 (5f94df478 2026-08-27)`. The installed stable toolchain was invoked directly because the checkout's named-toolchain invocation attempted to write to read-only rustup state.
- Installed executable on PATH: `uv 0.12.13 (x86_64-unknown-linux-gnu)`, checked with `uv --version`. It was not rebuilt or substituted. The reported methods are Rust library APIs, so this run evaluates them through a standalone Rust test harness, not through the installed CLI.
- Python 3.12.3 was available and used only to prepare the temporary files. The tests supply Python marker values `3.12.0` / `3.12`; no Python process is used for their evaluation.
- Harness, Cargo home, dependency downloads, lockfile, and build output are all under `/home/runner/work/_temp/uv-22259-0rxk_pva`. No checkout files or existing user state were modified, and no GitHub writes were made. No release build was performed.

### Minimal fixture and command

Create a standalone temporary Cargo project with empty `src/lib.rs` and this `Cargo.toml` (adjust the checkout path as needed):

```toml
[package]
name = "darwin-release-repro"
version = "0.0.0"
edition = "2024"

[dependencies]
uv-pep508 = { path = "/home/runner/work/uv/uv/crates/uv-pep508" }

[profile.dev]
debug = "line-tables-only"
```

The checkout's `Cargo.lock` was copied into the temporary project before running, to retain its dependency versions. The independently written `tests/darwin_release_fallback.rs` is:

```rust
use std::error::Error;

use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder, MarkerTree};

#[test]
fn darwin_unparseable_release_negation() -> Result<(), Box<dyn Error>> {
    let environment = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
        implementation_name: "cpython",
        implementation_version: "3.12.0",
        os_name: "posix",
        platform_machine: "arm64",
        platform_python_implementation: "CPython",
        platform_release: "not-a-version",
        platform_system: "Darwin",
        platform_version: "",
        python_full_version: "3.12.0",
        python_version: "3.12",
        sys_platform: "darwin",
    })?;
    let marker: MarkerTree = "platform_release == '24'".parse()?;
    println!(
        "equality={}, negation={}",
        marker.evaluate(&environment, &[]),
        marker.negate().evaluate(&environment, &[])
    );
    assert!(!marker.evaluate(&environment, &[]));
    assert!(marker.negate().evaluate(&environment, &[]));
    Ok(())
}

#[test]
fn invalid_darwin_release_is_not_a_numeric_tautology() -> Result<(), Box<dyn Error>> {
    let environment = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
        implementation_name: "cpython",
        implementation_version: "3.12.0",
        os_name: "posix",
        platform_machine: "arm64",
        platform_python_implementation: "CPython",
        platform_release: "3-invalid",
        platform_system: "Darwin",
        platform_version: "",
        python_full_version: "3.12.0",
        python_version: "3.12",
        sys_platform: "darwin",
    })?;
    let lower: MarkerTree = "platform_release >= '9'".parse()?;
    let upper: MarkerTree = "platform_release < '24'".parse()?;
    let marker: MarkerTree = "platform_release >= '9' or platform_release < '24'".parse()?;
    println!(
        "lower={}, upper={}, disjunction={}",
        lower.evaluate(&environment, &[]),
        upper.evaluate(&environment, &[]),
        marker.evaluate(&environment, &[])
    );
    println!("simplified={:?}", marker.try_to_string());
    assert!(!marker.evaluate(&environment, &[]));
    Ok(())
}
```

Exact invocation from the temporary project:

```sh
cd /home/runner/work/_temp/uv-22259-0rxk_pva
CARGO_HOME="$PWD/cargo-home" \
CARGO_TARGET_DIR="$PWD/target" \
RUSTC=/home/runner/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rustc \
/home/runner/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo \
  test --test darwin_release_fallback -- --nocapture
```

On a normally configured machine with the requested compiler available, the equivalent test command is `cargo +1.99.0 test --test darwin_release_fallback -- --nocapture` from that standalone project, with Cargo home and target directory redirected to temporary storage.

### Observed result

```text
running 2 tests
equality=false, negation=false
assertion failed: marker.negate().evaluate(&environment, &[])
test darwin_unparseable_release_negation ... FAILED
lower=false, upper=false, disjunction=true
simplified=Some("platform_release < '24' or platform_release >= '9' or sys_platform == 'darwin'")
assertion failed: !marker.evaluate(&environment, &[])
test invalid_darwin_release_is_not_a_numeric_tautology ... FAILED
test result: FAILED. 0 passed; 2 failed
```

The equality's logical negation should evaluate true. The disjunction should evaluate false when both operands evaluate false. For `3-invalid`, ordinary lexical comparisons also give false for both predicates because it sorts between `24` and `9`. The printed expression provides additional observed evidence that the release condition has disappeared for Darwin. Serialization/reparse round trips were not separately tested.

### Existing test coverage

Searched `crates/uv/tests/`, `crates/uv-client/tests/it/`, and `crates/uv-pep508/` for `platform_release` and the exact opaque values and reproduction names. No existing checkout test exercises these two opaque Darwin cases.

- `crates/uv-pep508/src/marker/tree.rs`, `darwin_platform_release`: inspected setup and assertions. Uses releases `24.0.0` and `24.10.0`; covers numeric equivalence with `24`, disjointness, ordering against `24.9.0`, negation, and printed round trips. It does not supply an unparseable release.
- `crates/uv/tests/lock/lock.rs`, `lock_required_environment_macos_release`: requires Darwin release `24.0.0` and uses macOS wheel tags to check numeric release splits and wheel availability. It is gated by `test-universal` and does not evaluate opaque values.
- The adjacent `lock_required_environment_macos_release_python_fork`, also gated by `test-universal`, checks that wheels for another Python version do not determine the Darwin release split. Its required release is again `24.0.0`.
- `crates/uv-client/tests/it/user_agent_version.rs`, `test_user_agent_has_linehaul`: constructs a Linux environment with release `6.5.0-1016-azure` to test user-agent metadata; it is not a Darwin marker-comparison test.

The temporary public-API harness needs no `test-*` feature gate or Python fixture. No repository tests were added or modified, and the existing suites were not run.

## Related

- astral-sh/uv#22272 — Preserve lexical fallback for Darwin release markers (open draft pull request). Draft opened in response to astral-sh/uv#22259. Proposes paired numeric and lexical constraints to address both reported failures, with negation, composition, and serialization tests. It preserves legacy lexical fallback; it is not a merged fix or an independent duplicate.
- astral-sh/uv#22295 — Use version zero for unparseable Darwin releases (open draft pull request). Alternative draft responding to astral-sh/uv#22259. Maps unparseable releases to version zero to restore consistency with numeric range algebra. This changes the fallback contract: the reported union remains true, rather than satisfying the requested opaque-string result.
- astral-sh/uv#22258 — test: reproduce Darwin release marker fallback inconsistencies (open draft pull request). Companion reproduction-only draft. Its diff adds exactly the two reported public-API tests and no production changes; the reported run has two failures. It does not resolve the issue.
- astral-sh/uv#21766 — Infer Darwin release markers from macOS wheel tags (merged pull request). Merged September 17, 2026. Its diff introduced Darwin-specific VersionString nodes, numeric range lowering, and the parse-error return false branch implicated here. Its tests cover valid dotted releases, not opaque values.
- astral-sh/uv#22304 — Set Darwin release markers for macOS targets (open draft pull request). Related draft supplies a Darwin release baseline for explicit macOS targets, whose release is currently empty. This addresses a concrete CLI trigger of the same evaluation path, but leaves arbitrary opaque-string evaluation and simplification separate.
- astral-sh/uv#19808 — Update string marker ordering semantics (merged pull request). Merged June 13, 2026. Implements current ordering rules for pure String marker fields, explicitly excluding platform_release and platform_version because they are Version | String fields. Relevant contract precedent, not a previous fix of these failures.
- astral-sh/uv#3917 — Different understanding of environment markers vs pip (closed issue). Historical discussion of non-version platform_release values and lexical versus PEP 440 comparisons; maintainer comments explain the fallback ambiguity. Its Linux/pip comparison predates Darwin VersionString lowering and does not track the negation or numeric-union failures.

## Supporting evidence

Source inspection used the same checkout as the independent reproduction, `01b62808962d7abfe2d10f43d652f357d8038202`.

- `crates/uv-pep508/src/marker/environment.rs:301`: `MarkerEnvironmentBuilder` converts `platform_release` directly to a string while parsing actual version fields. Environment construction accepted both opaque release values during the run.
- `crates/uv-pep508/src/marker/tree.rs:836`: `negate()` is documented as logical negation and complements the node. At line 1094, the `VersionString` evaluator returns false immediately when parsing the environment value as a version fails, before following a child edge. This matches the observed false equality and false negation.
- `crates/uv-pep508/src/marker/algebra.rs:324`: a parseable release constant is lowered to numeric `VersionString` edges in the Darwin branch, with string edges restricted to non-Darwin environments. This decision does not depend on whether the runtime release is parseable.
- `crates/uv-pep508/src/marker/algebra.rs:1445` and line 1588: numeric specifiers become ranges and composition merges ranges. The union of `>= 9` and `< 24` covers the numeric domain. `create_node` at line 131 removes a parent when every child is the same. Consistent with this path, the observed printed disjunction contains an unconditional Darwin branch; changing only the parse-error return would not retain the discarded condition.
- `crates/uv-pep508/src/marker/simplify.rs:94`: printing reconstructs string expressions from numeric bounds. A repair should be checked through printing/serialization as well as direct evaluation.
- The previously inspected diff of astral-sh/uv#22295 expects `3-invalid < 24` to be true by mapping it to version zero. It restores consistency with numeric algebra but differs from the report's requested opaque-string result. astral-sh/uv#22272 instead retains a false union and adds round-trip coverage.
- `crates/uv-configuration/src/target_triple.rs:1533`: explicit macOS and Apple Darwin targets supply an empty release. astral-sh/uv#22304 proposes a Darwin baseline derived from the deployment target. This separate CLI trigger was not part of the independent reproduction.

The related draft review found no maintainer decision establishing an accepted fallback policy. Neither proposed policy should be described as an accepted or released fix.

## Draft response

Both examples reproduce with Rust 1.99.0 against the current checkout. With Darwin explicitly selected, the equality and its logical negation both return false for `not-a-version`; for `3-invalid`, each comparison returns false but their disjunction returns true. The printed disjunction includes an unconditional Darwin branch. These results confirm the public-API inconsistencies without requiring an ordinary macOS release or an installation scenario.

astral-sh/uv#22272 proposes lexical fallback through composition and serialization. astral-sh/uv#22295 proposes version zero for unparseable releases, giving the second example a different intended result. Both remain drafts. The maintainer decision is the fallback contract; review the chosen design against negation, disjunction, and printing/serialization round trips. Keep explicit macOS target population distinct from the general public-API behavior.

## Search coverage and excluded candidates

The prior context review searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separate literal searches covered platform_release, not-a-version, 3-invalid, marker.negate().evaluate, darwin_release_fallback, and VersionString. Conceptual searches covered negation, boolean logic, tautologies/always-true markers, simplification, lexical/lexicographic and string comparisons, serialization, and area:rustlib. Fix-oriented searches covered Darwin release inference and string-ordering changes. REST PR searches hit a rate limit and indexed PR searches omitted known matches; supplemented them by filtering titles/bodies of the latest 3,000 PRs across all states, then inspecting relevant diffs, comments, reviews, and referenced discussions. Ruled out astral-sh/uv#12833 and astral-sh/uv#21309 and astral-sh/uv#21310 (invalid literal comparisons or version containment), astral-sh/uv#18971 and astral-sh/uv#19105 (dependency selection on ordinary macOS releases), astral-sh/uv#5044 and astral-sh/uv#5078 and astral-sh/uv#6295 leading to astral-sh/uv#5992 (simplification completeness/conciseness), and astral-sh/uv#22262 with merged astral-sh/uv#22234 (export extra-activation propagation).

The report was decomposed before searching into failed logical negation, incorrect disjunction after simplification, and preservation through printing/serialization. The subsystem is the public marker API, with Darwin selected in the supplied environment and a release value that cannot be parsed as a version. Searches for those observable failures were kept separate from searches for numeric-range lowering as a possible cause. Repository labels and maintainer terminology informed the Rust-library, comparison, and simplification searches.

The closest excluded discussions have materially different triggers:

- astral-sh/uv#12833 concerns two literal operands, which maintainers describe as an invalid marker form that uv ignores. astral-sh/uv#21309 and astral-sh/uv#21310 concern `in` on version fields; the linked astral-sh/uv#21311 discusses deprecated containment syntax. This report uses supported comparisons against an environment field.
- astral-sh/uv#18971 and its follow-up astral-sh/uv#19105 concern dependency selection with normal macOS releases. In the closed original, a maintainer showed that the chosen version satisfied published metadata and suggested environment constraints.
- astral-sh/uv#5044 and astral-sh/uv#5078 concern missing simplifications of genuine Boolean tautologies. astral-sh/uv#6295 leads to the canonical conciseness discussion in astral-sh/uv#5992; maintainers explicitly distinguish correct but verbose markers. This report concerns a changed truth value.
- astral-sh/uv#22262 initially looks relevant because export drops platform conditions. A maintainer identifies merged astral-sh/uv#22234 as its fix; that change tracks optional-extra activation separately from package reachability, not Darwin version/string comparisons.

The historical comments in astral-sh/uv#3917 were followed to pypa/packaging#774, which concerns Linux `InvalidVersion` failures, and the specification background in astral-sh/uv#19808 was followed to merged pypa/packaging.python.org#1988. These explain the changing comparison contract; neither establishes a prior uv fix for the reported Darwin algebra problem.

Search completeness is limited by the REST search rate limit and PR index omissions. Direct PR enumeration across all states recovered the reproduction, both competing repairs, the explicit-target proposal, and the introducing and string-ordering changes. No independent duplicate was found among the inspected results.
