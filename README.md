# .env parsing - dotenvy fails to correctly parse files with spaces and some special characters

Issue: astral-sh/uv#23226

Classification: bug

## Summary

The reporter describes application breakage when migrating from python-dotenv to uv's command-line dotenv injection on uv 0.13.6, with Python 3.14.10 on macOS and Linux. They identify four concerns and offer to document the differences rather than replace dotenvy:

- Dollar signs can trigger substitution and silently change a value.
- A bare `$A_PLAIN` reference is read as `$A` plus the literal suffix `_PLAIN`.
- Unquoted values containing spaces are rejected.
- These incompatibilities with other dotenv implementations are insufficiently documented.

No concrete uv command or complete minimal reproduction accompanies the report. The relevant subsystem is dotenv loading for spawned commands, including `uv run --env-file`. The released uv 0.13.6 lockfile and this checkout both use dotenvy 0.15.7. The pinned upstream parser confirms the reported underscore behavior, substitution semantics, and whitespace restriction. A new runtime reproduction is not required to establish the underscore correctness issue.

The closest uv history is astral-sh/uv#8862, whose comments include an earlier substitution failure involving an underscore-containing variable. It was closed around the separate topic of opt-in loading, not a parsing fix. The reporter's four upstream leads were independently checked and support the distinct claims. No canonical uv duplicate or fix for the reported parsing behavior was found.

## Draft response

uv 0.13.6 uses dotenvy 0.15.7, whose parser confirms these differences. The underscore-substitution case is a correctness issue tracked in allan2/dotenvy#170: use `${A_PLAIN}` as a workaround for `$A_PLAIN`. Quote values containing spaces, and single-quote values when dollar signs should be literal; double quotes still allow substitution.

The current documentation names dotenvy but does not explain these compatibility differences. If you'd like to contribute a focused documentation PR, please add examples to the “Environment variable files” section of docs/concepts/configuration-files.md and reference the upstream issues. One distinction to document: uv emits a warning on a parse error and stops reading the rest of that file, whereas substitution can successfully parse and change a value without a warning.

## Classification

The underscore-substitution error is confirmed in the dotenvy 0.15.7 source pinned by uv 0.13.6: valid underscore-containing keys are accepted, but bare substitution references stop at the underscore and can silently produce the wrong value. That is a correctness problem even though the reporter primarily requests documentation. Dollar substitution and stricter whitespace handling also need clear compatibility guidance. The upstream issues track dependency behavior, while astral-sh/uv#8862 was closed around opt-in loading rather than a parsing fix; none centrally tracks this uv report. No previously fixed regression was established.

Known or deliberately retained dependency behavior does not make incorrect variable substitution correct. The parser accepts underscores in variable definitions but excludes them from bare substitution names, producing a different value without an error. Documentation is a useful immediate action, but this report includes a confirmed correctness problem and therefore merits the bug classification rather than enhancement or question.

The upstream issues are relevant dependency discussions, not a replacement for tracking uv's integration and user-facing documentation. The historical uv support issue was redirected to an auto-loading discussion; it does not provide a canonical discussion of these parsing problems. There is no evidence that a previously released uv fix addressed this problem and later regressed.

## Related

- astral-sh/uv#8862 — uv run does not use env-var in .env-file (closed). A reporter comment in astral-sh/uv#8862 describes substitution of an underscore-containing variable defined earlier in the same .env file failing while other tools worked. This is relevant history, but the issue was closed in favor of astral-sh/uv#16926 about opt-in loading, without a parsing fix.
- allan2/dotenvy#170 — Inconsistent behavior when substituting variables with underscore in their names (open). allan2/dotenvy#170 reproduces the exact underscore-substitution symptom: FOO_BAR=test followed by FOO_BAZ=$FOO_BAR yields _BAR. The dotenvy 0.15.7 source used by uv 0.13.6 confirms that bare substitution names accept alphanumeric characters but stop at underscores.
- allan2/dotenvy#93 — dotenvy silently fails to parse strings with dollar signs (closed). allan2/dotenvy#93 demonstrates dollar-containing values changing during parsing. It was closed as intended variable-substitution behavior, with maintainer acknowledgment; a later comment requests documentation. This supports the dollar-sign compatibility concern, rather than establishing a fixed bug.
- allan2/dotenvy#11 — Unquoted values cannot contain spaces (open). allan2/dotenvy#11 gives the same unquoted-space example, ABC=foo bar. The maintainer confirms the stricter parser behavior, recommends quoting or escaping whitespace, and identifies documentation of implementation differences as useful.
- allan2/dotenvy#19 — Document differences across dotenv implementations (open). allan2/dotenvy#19 tracks upstream documentation of quoting, escaping, spaces, and substitution differences. It supports the requested compatibility documentation, but does not cover uv's own documentation or resolve the underscore correctness problem.

## Supporting evidence

### Released dependency and uv behavior

- [uv 0.13.6 Cargo.lock](https://github.com/astral-sh/uv/blob/0.13.6/Cargo.lock) pins dotenvy 0.15.7. The checkout also declares it in `Cargo.toml:170` and locks that version in `Cargo.lock:1490`. The checkout identifies uv as 0.13.7, so the released 0.13.6 lockfile was checked separately.
- [uv 0.13.6 dotenv reader](https://github.com/astral-sh/uv/blob/0.13.6/crates/uv-command-support/src/child.rs#L15) calls `dotenvy::from_path_iter`. Parse errors emit a warning; an error returned during iteration breaks the file's read loop. Values parsed before that point remain in the child-process overlay. Successfully parsed substitutions produce no corresponding warning.
- The same reader in `crates/uv-command-support/src/child.rs:15` is used by `crates/uv-project-commands/src/run.rs:174` and `crates/uv-tool-commands/src/run.rs:192`. Existing environment variables take precedence over file values. That is a separate mechanism from malformed substitutions.
- `crates/uv/tests/project/run.rs:5184`, `run_with_malformed_env`, already snapshots a successful command exit with a parse warning and an unset malformed variable. The inspected dotenv test block covers loading, missing files, multiple files, disabling loading, and malformed input; it does not specifically cover the three reported syntax cases.
- `docs/concepts/configuration-files.md:80` identifies dotenvy, demonstrates a quoted value with spaces, and documents precedence and file selection. It does not explain dollar-sign substitution, underscore references, or the requirement to quote or escape embedded whitespace.

### Pinned parser behavior

The [dotenvy 0.15.7 parser](https://github.com/allan2/dotenvy/blob/e330110a7cd9064ec913e554c9fc325c9d7cdb88/dotenv/src/parse.rs) establishes:

- `parse_key` accepts underscores. In `parse_value`, bare substitution names accumulate only `is_alphanumeric()` characters. An underscore terminates the name, applies the shorter substitution, and becomes literal output. The upstream test `variable_without_parenthesis_is_substituted_before_separators` explicitly expects this behavior.
- Braced references accumulate underscores until the closing brace, so `${A_PLAIN}` is the appropriate workaround for substitution of an underscore-containing name.
- Single-quoted values bypass substitution. Double-quoted and unquoted values allow it. Undefined substitutions become empty strings, explaining value changes without a parse error.
- Unquoted embedded whitespace followed by non-comment text causes `LineParse`. Quoted or escaped whitespace is supported.

These are source findings, not assumptions about shell expansion or a particular application. No claim is made that every dotenv implementation has identical semantics, or that an upstream release or parser replacement is scheduled.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separate literal searches covered dotenvy, unquoted values, dollar signs, A_PLAIN, underscore substitution, python-dotenv, and the exact 'Failed to parse environment file' diagnostic; conceptual searches covered env-file parsing, whitespace, interpolation, escaping, and documentation differences. Closed-issue and merged-PR searches looked for earlier fixes. Inspected candidate comments and reference chains, including astral-sh/uv#8862 to astral-sh/uv#16926 and astral-sh/uv#1384 to astral-sh/uv#8263. Ruled out astral-sh/uv#9465 (precedence), astral-sh/uv#16107 (reporter confirmed a VS Code extension caused the corruption), and astral-sh/uv#15806 with merged astral-sh/uv#15815 and open astral-sh/uv-dev#156 (spaces in file paths, not values). Verified the four reporter-cited upstream issues and the pinned parser source. Some exploratory searches hit rate limits; subsequent targeted searches succeeded. No uv parsing fix or canonical duplicate was found.

Especially plausible candidates were inspected and excluded from the related list because they address different behavior:

- astral-sh/uv#9465 requests configurable precedence between the process environment and dotenv files. Comments discuss migration surprises and quoting differences, but it does not track the reported parsing defects.
- astral-sh/uv#15806 concerns whitespace in the path supplied to `--env-file`. Its historical fix, astral-sh/uv#15815, merged on 2025-09-12 and added escaping to path-list handling. The linked open astral-sh/uv-dev#156 also addresses explicit path splitting. None changes parsing of values inside a dotenv file.
- astral-sh/uv#16107 looks close because it reports corrupted dotenv values containing dashes and spaces, but its author confirmed that VS Code's Python environment extension caused the corruption and that another terminal worked.
- astral-sh/uv#1384 and merged astral-sh/uv#8263 concern initial dotenv support; later maintainer comments clarify that loading became opt-in before release. The follow-on astral-sh/uv#16926, astral-sh/uv#9994, and astral-sh/uv#16672 concern automatic loading, persistent configuration, or missing files.
- Upstream allan2/dotenvy#19 links to allan2/dotenvy#82 about backslash escaping and open allan2/dotenvy#139 about shell parameter modifiers. Those are adjacent syntax concerns, not a released fix for the reported underscore behavior.

## Maintainer next step

Keep the issue classified as a bug. A focused documentation contribution can explain quoting, literal dollar signs, braced substitutions, and the difference between parse warnings and successful substitutions that change values. Keep the underscore correctness concern explicit and linked to its upstream tracker. Any later regression test should use synthetic values and a targeted command in the existing dotenv test module; a real application dotenv file is unnecessary.

## Validation and scope

The assessment uses the released dependency lockfile, pinned parser source and its existing tests, uv's reader and existing snapshots, and issue/PR discussions. No builds or tests were run, and no runtime reproduction was claimed. Only this temporary maintainer handoff was authored; checkout files and GitHub state were not changed.
