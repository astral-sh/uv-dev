# Generated POSIX console launchers interpret destination paths as shell syntax

Issue: astral-sh/uv#23057

Classification: bug

## Summary

The reporter describes a console-entrypoint launch failure with uv 0.13.6 on Fedora Linux. After creating a virtual environment under a directory containing a literal tab and installing a wheel with a `console_scripts` entrypoint using `uv pip install --python`, executing the installed launcher fails with `bad interpreter: No such file or directory`.

The relevant condition is a non-relocatable environment whose full interpreter path contains a tab but no space, and whose complete shebang line is at most 127 bytes long. Otherwise, another condition can select the shell wrapper and mask the missing tab check. The expected behavior is for the entrypoint to run using that environment's interpreter.

astral-sh/uv#2076 and merged astral-sh/uv#2097 explain the existing space/length fallback and its omission of tabs. Open astral-sh/uv#22323 concerns a separate consumer of shell wrappers. No existing report or fix for the tab-specific failure was found.

The title suggests shell interpretation, while the supplied reproduction concerns kernel parsing of a direct shebang. The reporter cites a virtualenv change and describes prior security correspondence; this handoff establishes the launcher correctness issue without independently establishing a vulnerability or arbitrary shell execution.

## Draft response

The tab case is a bug: `format_shebang` only selects the POSIX shell wrapper for spaces, long shebangs, or relocatable environments. A short interpreter path containing a tab therefore gets a direct shebang that the kernel splits incorrectly. The earlier fix in astral-sh/uv#2097 covered spaces and long paths but omitted tabs.

The next step is to include tabs in the fallback condition and add a regression test that installs and runs a console entrypoint under a short path containing a literal tab and no spaces. The reproduction establishes a shebang-parsing failure; it does not establish shell execution of path contents.

## Classification

The checked-in uv 0.13.6 source confirms that a non-relocatable POSIX interpreter path containing a tab, but no space and with a shebang line at most 127 bytes long, receives a direct shebang. The kernel splits that path at the tab, yielding an unusable console launcher. The historical space/length fix never covered tabs, and no existing discussion was found that tracks this remaining case. This is incorrect launcher generation, not an established regression or evidence of arbitrary shell execution.

The report provides enough information to identify the missing fallback condition from source. No additional reproduction request is needed for classification. The closed historical issue and merged fix specifically addressed spaces and length; they do not establish that tab-containing paths previously worked.

## Related

- astral-sh/uv#2076 — Support paths with spaces in launchers (closed; issue). Historical counterpart: POSIX console launchers failed because the kernel split interpreter paths containing spaces. It requested the shell-wrapper fallback, implemented by astral-sh/uv#2097. Its space-path case was fixed; the newly reported tab condition remains outside that fix.
- astral-sh/uv#2097 — Wrap unsafe script shebangs in `/bin/sh` (merged; pull request). Merged on 2024-02-29 and closed astral-sh/uv#2076. The diff introduced the same fallback predicate still present today: shebang length over 127 bytes or a literal space. Tabs were never included, establishing an omitted case rather than a regression of tab support.
- astral-sh/uv#22323 — `uv run --with` packages invisible to project entrypoints when the venv path forces the /bin/sh shebang wrapper (open; issue). Adjacent issue involving the same generated wrapper. Its report and follow-up describe valid wrappers for long or space-containing paths being skipped when copying entrypoints into an ephemeral environment. Here, a short tab-containing path does not receive a wrapper at all and the launcher cannot start; these require different fixes.

## Supporting evidence

- In `crates/uv-install-wheel/src/wheel.rs:128`, `format_shebang` checks POSIX shebang length as `2 + executable.len() + 1`. At line 141, its fallback predicate is `shebang_length > 127 || executable.contains(' ') || relocatable`. A tab alone does not satisfy it; the function returns a direct `#!` interpreter path at line 147.
- In the same file at line 385, console-entrypoint generation calls `format_shebang`, connecting this predicate directly to the reported installed launchers.
- When selected, the wrapper branch already single-quotes the executable and calls `escape_posix_for_single_quotes`. That helper, in `crates/uv-shell/src/shlex.rs:33`, escapes embedded single quotes. This evidence supports distinguishing the missing tab fallback from an assertion that all destination paths are interpolated as shell syntax.
- The existing `test_shebang` at `crates/uv-install-wheel/src/wheel.rs:1428` covers ordinary paths, spaces, relocation, Windows, quotes, and long paths, but has no tab case. The inspected pip-install tests include shell-wrapper encoding cases; they do not exercise the short tab-containing console-entrypoint path.
- The original diff of astral-sh/uv#2097 contains the literal-space and length checks, without a tab check. It merged on 2024-02-29. A maintainer also identified it as the fix for astral-sh/uv#1466. The current omission therefore predates the reported 0.13.6 version.
- The reporter-linked pypa/virtualenv commit `2519941bed33ee0d3ac99277f67913e439e476a9`, dated 2026-10-17, explicitly adds tab-path shell-launcher handling alongside interpreter-path quoting changes. It is useful comparative evidence, but a fix in virtualenv does not establish a fix or identical security impact in uv.
- astral-sh/uv#22526 merged on 2026-10-14 and is recorded in the checked-in changelog. It fixes astral-sh/uv#21276 by preserving wheel data-script encoding declarations and protecting paths from source-code decoding. That fix concerns scripts with encoding declarations, not the tab-trigger predicate for generated console entrypoints.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separately searched tab-path launch failures and shell interpretation: bad interpreter, shebang, console_scripts, tab/venv, CVE-2026-113722, format_shebang, format_shell_shebang, whitespace, special characters, quoting, escaping, and command substitution, including comments and installer/environment labels. Removed version and platform restrictions. PR keyword searches returned no hits, so also screened the latest 1,000 PRs across states and directly inspected referenced historical fixes. Followed astral-sh/uv#1466 to astral-sh/uv#2097. Ruled out astral-sh/uv#2744 and astral-sh/uv#8035 (setuptools build-interpreter paths), astral-sh/uv#8947 (activation quoting), astral-sh/uv#9348 (upstream bundled scripts), astral-sh/uv#16209 (BusyBox realpath), and astral-sh/uv#21276 with astral-sh/uv#22526 (wheel data-script encoding). Also inspected the reporter-linked virtualenv commit; no existing tab-specific uv fix or duplicate was found.

The strongest excluded matches differ in the failure mechanism:

- astral-sh/uv#2744 and its duplicate astral-sh/uv#8035 describe setuptools editable scripts pointing to a temporary build interpreter. The discussion explicitly distinguishes those scripts from console entrypoints.
- astral-sh/uv#8947 concerns apostrophes in activation scripts, not console launcher generation.
- astral-sh/uv#9348 concerns shell wrappers shipped in python-build-standalone; maintainer comments direct the fix upstream.
- astral-sh/uv#16209 concerns BusyBox interpreting `realpath --` incorrectly in relocatable wrappers. Maintainer comments state that the launcher still functions despite the warning.
- astral-sh/uv#21276 and astral-sh/uv#22526 concern Python source encoding after insertion of a multi-line wrapper, not interpreter-path splitting by the kernel.

Searches covered both the literal tab/error report and the title's broader shell-syntax claim. Conceptual searches used whitespace, special characters, launcher quoting, escaping, and command substitution; searches using source identifiers and historical fixes were assessed separately. Searches were not restricted to Fedora, a package name, or uv 0.13.6. PR keyword searches returned no results even for known historical fixes, so the handoff also relies on direct linked-PR inspection and screening the latest 1,000 PRs across states; this is not a claim that every historical PR was enumerated.

## Suggested next step and verification limits

Extend POSIX fallback selection to include literal tabs. Add a focused Unix integration test in the existing pip-install module that installs a wheel with a console entrypoint and executes it from a non-relocatable environment under a tab-containing directory. Ensure the full interpreter path has no spaces and stays below the length fallback threshold so the test isolates the missing condition. Confirm that the launched interpreter is the intended environment's interpreter.

Findings are based on source, existing tests, issue discussions, and PR diffs. No reproduction, build, or test suite was run. No checkout files or GitHub objects were changed; only this temporary maintainer handoff was authored.
