# Darwin release markers lose opaque-string semantics during evaluation and simplification

Issue: astral-sh/uv#22259

Classification: bug

Status checked: October 9, 2026. The issue is open. The four related open pull requests are drafts; none is a merged repair.

## Summary

astral-sh/uv#22272 and astral-sh/uv#22295 are unmerged draft proposals with different fallback policies; astral-sh/uv#22258 supplies the reproductions. astral-sh/uv#21766 introduced the affected path, and astral-sh/uv#22304 addresses explicit macOS targets. Earlier string-ordering discussions provide context, not a duplicate.

The report concerns two public `uv-pep508` API inconsistencies with a supplied Darwin marker environment:

- With `platform_release = "not-a-version"`, `platform_release == '24'` evaluates false and its logical negation also evaluates false. The negation should evaluate true.
- With `platform_release = "3-invalid"`, `platform_release >= '9' or platform_release < '24'` evaluates true after numeric range reduction. Both comparisons would be false under lexical fallback: `3-invalid` sorts between `24` and `9`. They are also false under the current specification's opaque-string ordering rules.

The reporter explicitly uses synthetic API inputs and does not claim failure on ordinary macOS releases or demonstrate an end-to-end installation failure. The reported source base is `eaa0fb829a581ef50fd75c53b5ca964bc73d31c6`; the reproduction-only commit is `946e286a63dc6972e45bc2f43a80301439f2d7b7`, with Rust 1.99.0 on Darwin arm64. Python is not invoked.

## Draft response

Your examples identify two correctness problems in the public marker API. The source confirms that failed version parsing bypasses negation, and numeric range simplification can erase the opaque-string case before evaluation.

astral-sh/uv#22272 proposes preserving lexical fallback through composition and serialization. astral-sh/uv#22295 instead proposes treating unparseable releases as version zero, which would give your second example a different result. Both remain drafts. The next step is to settle the fallback contract and review it against both reproductions and printing/serialization round trips.

## Classification

Source confirms incorrect behavior for opaque strings accepted by MarkerEnvironment: failed VersionString parsing bypasses complemented edges, while numeric-only range reduction can eliminate the opaque domain before evaluation. The broader fallback policy remains undecided, but these correctness failures are established. The companion reproduction and subsequent proposed fixes respond to this issue, so they do not make it a duplicate. No previous fix of these failures was established; astral-sh/uv#19808 explicitly excluded these fields.

The issue was opened on October 6, 2026. astral-sh/uv#22258 is its companion reproduction, created minutes earlier; its body explicitly identifies this issue and says it contains intentionally failing tests. astral-sh/uv#22272 and astral-sh/uv#22295 were opened on October 7 and explicitly propose closing this issue. These are follow-up proposals, not independent canonical discussions.

astral-sh/uv#21766 introduced the implicated code on September 17, 2026. The June string-ordering change in astral-sh/uv#19808 deliberately left Version | String fields outside its scope. The evidence therefore does not establish recurrence of a previously fixed bug. The earlier closed fallback discussion does not resolve the concrete logical inconsistencies reported here.

## Related

- astral-sh/uv#22272 — Preserve lexical fallback for Darwin release markers (open draft pull request). Draft opened in response to astral-sh/uv#22259. Proposes paired numeric and lexical constraints to address both reported failures, with negation, composition, and serialization tests. It preserves legacy lexical fallback; it is not a merged fix or an independent duplicate.
- astral-sh/uv#22295 — Use version zero for unparseable Darwin releases (open draft pull request). Alternative draft responding to astral-sh/uv#22259. Maps unparseable releases to version zero to restore consistency with numeric range algebra. This changes the fallback contract: the reported union remains true, rather than satisfying the requested opaque-string result.
- astral-sh/uv#22258 — test: reproduce Darwin release marker fallback inconsistencies (open draft pull request). Companion reproduction-only draft. Its diff adds exactly the two reported public-API tests and no production changes; the reported run has two failures. It does not resolve the issue.
- astral-sh/uv#21766 — Infer Darwin release markers from macOS wheel tags (merged pull request). Merged September 17, 2026. Its diff introduced Darwin-specific VersionString nodes, numeric range lowering, and the parse-error return false branch implicated here. Its tests cover valid dotted releases, not opaque values.
- astral-sh/uv#22304 — Set Darwin release markers for macOS targets (open draft pull request). Related draft supplies a Darwin release baseline for explicit macOS targets, whose release is currently empty. This addresses a concrete CLI trigger of the same evaluation path, but leaves arbitrary opaque-string evaluation and simplification separate.
- astral-sh/uv#19808 — Update string marker ordering semantics (merged pull request). Merged June 13, 2026. Implements current ordering rules for pure String marker fields, explicitly excluding platform_release and platform_version because they are Version | String fields. Relevant contract precedent, not a previous fix of these failures.
- astral-sh/uv#3917 — Different understanding of environment markers vs pip (closed issue). Historical discussion of non-version platform_release values and lexical versus PEP 440 comparisons; maintainer comments explain the fallback ambiguity. Its Linux/pip comparison predates Darwin VersionString lowering and does not track the negation or numeric-union failures.

## Supporting evidence

Source inspection used checkout `01b62808962d7abfe2d10f43d652f357d8038202`.

- `crates/uv-pep508/src/marker/environment.rs:301`: `MarkerEnvironmentBuilder` converts `platform_release` directly to a string while parsing the actual version fields. Opaque release inputs are accepted, rather than rejected during environment construction. The setter at line 220 also accepts arbitrary strings.
- `crates/uv-pep508/src/marker/tree.rs:836`: the public `negate()` contract returns the logical negation by complementing the node. At line 1094, the `VersionString` evaluator returns false immediately on a failed version parse, before following a child edge that incorporates the complement.
- `crates/uv-pep508/src/marker/algebra.rs:324`: a parseable release constant is lowered to numeric `VersionString` edges in the Darwin branch, with string edges restricted to non-Darwin environments. The decision does not depend on whether the runtime environment's release is parseable.
- `crates/uv-pep508/src/marker/algebra.rs:1445` and line 1588: numeric specifiers become version ranges, and composition merges intersecting ranges. The union of `>= 9` and `< 24` covers the numeric domain. `create_node` at line 131 removes a parent when every child is the same, so the Darwin release check can disappear before runtime evaluation. Changing only the parse-error return cannot retain the lost opaque case.
- `crates/uv-pep508/src/marker/simplify.rs:94`: printing reconstructs string marker expressions from numeric version bounds. A repair must preserve the chosen opaque semantics through simplification and printing/serialization, not only direct evaluation.
- `crates/uv-pep508/src/marker/tree.rs:1888`: the existing Darwin test covers valid numeric releases, negation, and printed round trips. It does not cover the two opaque values in the report. The diff of astral-sh/uv#22258 adds two separate public-API tests and no production code.
- The diff of astral-sh/uv#22295 explicitly expects `3-invalid < 24` to be true by mapping it to version zero. It restores consistency with numeric algebra but does not satisfy the report's requested opaque-string result. The tests in astral-sh/uv#22272 instead retain the false union and exercise round trips.
- `crates/uv-configuration/src/target_triple.rs:1533`: explicit macOS and Apple Darwin targets currently supply an empty release. astral-sh/uv#22304 proposes deriving a Darwin baseline from the deployment target. This is a separate CLI trigger documented by the follow-up proposals, not an ordinary-host macOS failure demonstrated by the original report.

The latest related drafts have no maintainer reviews establishing an accepted fallback policy. astral-sh/uv#22272 retains legacy lexical ordering; astral-sh/uv#22295 chooses a numeric sentinel. Neither choice should be described as an accepted or released fix.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separate literal searches covered platform_release, not-a-version, 3-invalid, marker.negate().evaluate, darwin_release_fallback, and VersionString. Conceptual searches covered negation, boolean logic, tautologies/always-true markers, simplification, lexical/lexicographic and string comparisons, serialization, and area:rustlib. Fix-oriented searches covered Darwin release inference and string-ordering changes. REST PR searches hit a rate limit and indexed PR searches omitted known matches; supplemented them by filtering titles/bodies of the latest 3,000 PRs across all states, then inspecting relevant diffs, comments, reviews, and referenced discussions. Ruled out astral-sh/uv#12833 and astral-sh/uv#21309 and astral-sh/uv#21310 (invalid literal comparisons or version containment), astral-sh/uv#18971 and astral-sh/uv#19105 (dependency selection on ordinary macOS releases), astral-sh/uv#5044 and astral-sh/uv#5078 and astral-sh/uv#6295 leading to astral-sh/uv#5992 (simplification completeness/conciseness), and astral-sh/uv#22262 with merged astral-sh/uv#22234 (export extra-activation propagation).

The report was decomposed before searching into failed logical negation, incorrect disjunction after simplification, and preservation through printing/serialization. The subsystem is the public marker API, with Darwin selected in the supplied environment and a release value that cannot be parsed as a version. Searches for those observable failures were kept separate from searches for numeric-range lowering as a possible cause. Repository labels and maintainer terminology informed the Rust-library, comparison, and simplification searches.

The closest excluded discussions have materially different triggers:

- astral-sh/uv#12833 concerns two literal operands, which maintainers describe as an invalid marker form that uv ignores. astral-sh/uv#21309 and astral-sh/uv#21310 concern `in` on version fields; the linked astral-sh/uv#21311 discusses deprecated containment syntax. This report uses supported comparisons against an environment field.
- astral-sh/uv#18971 and its follow-up astral-sh/uv#19105 concern dependency selection with normal macOS releases. In the closed original, a maintainer showed that the chosen version satisfied published metadata and suggested environment constraints.
- astral-sh/uv#5044 and astral-sh/uv#5078 concern missing simplifications of genuine Boolean tautologies. astral-sh/uv#6295 leads to the canonical conciseness discussion in astral-sh/uv#5992; maintainers explicitly distinguish correct but verbose markers. This report concerns a changed truth value.
- astral-sh/uv#22262 initially looks relevant because export drops platform conditions. A maintainer identifies merged astral-sh/uv#22234 as its fix; that change tracks optional-extra activation separately from package reachability, not Darwin version/string comparisons.

The historical comments in astral-sh/uv#3917 were followed to pypa/packaging#774, which concerns Linux `InvalidVersion` failures, and the specification background in astral-sh/uv#19808 was followed to merged pypa/packaging.python.org#1988. These explain the changing comparison contract; neither establishes a prior uv fix for the reported Darwin algebra problem.

Search completeness is limited by the REST search rate limit and PR index omissions. Direct PR enumeration across all states recovered the reproduction, both competing repairs, the explicit-target proposal, and the introducing and string-ordering changes. No independent duplicate was found among the inspected results.

## Validation and next step

The reported command is `cargo +1.99.0 test -p uv-pep508 --test darwin_release_fallback` on the reproduction branch. Its reported result is zero passed and two failed, at the negated-equality and numeric-tautology assertions. That run was not independently repeated; the findings here are supported by source and diff inspection. No checkout changes or GitHub writes were made.

The maintainer decision is the fallback contract for opaque Darwin releases. Review the selected design against both reported cases, composition and negation consistency, and printing/serialization round trips. Keep explicit macOS target population distinct from the general public-API behavior. No additional reproduction information is needed to classify these correctness failures.
