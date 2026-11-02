# virtualenv activate runs commands from the environment name in zsh with PROMPT_SUBST

Issue: astral-sh/uv#23059

Classification: duplicate

## Summary

The reporter describes command substitution in a custom virtual-environment prompt executing when the generated activation script is sourced in zsh, using uv 0.13.6 on Fedora Linux with any Python version. The example puts a shell command substitution in `uv venv --prompt`, sources `venv/bin/activate` in a noninteractive zsh process, and checks for the command's side effect.

astral-sh/uv#23058 is the canonical match for the reported source-time execution. astral-sh/uv#9424 and merged astral-sh/uv#8984 provide historical path-quoting context, not a prior fix for this prompt bug.

## Draft response

Your reproducer hits the same prompt-escaping bug tracked in astral-sh/uv#23058, which already covers bash and zsh. The generated activation script inserts the prompt directly into double-quoted shell expressions, so command substitution runs while sourcing it, without requiring PROMPT_SUBST or a prompt redraw. Please keep the zsh reproduction and PROMPT_SUBST follow-up in astral-sh/uv#23058 so activation-time escaping and prompt rendering can be addressed together.

## Classification

The incorrect behavior is already tracked by open astral-sh/uv#23058, which explicitly covers source-time command substitution in both bash and zsh using the same prompt and uv version. The supplied reproduction does not isolate a separate PROMPT_SUBST redraw bug. Duplicate takes precedence over bug; the historical path-quoting fix in astral-sh/uv#8984 did not fix prompt interpolation, so the available evidence does not establish a regression.

The report describes incorrect behavior, independent of its security classification. The reporter says Astral security reviewed it as outside uv's threat model; this handoff does not independently verify that private correspondence or assert that the upstream CVE identifiers apply to uv.

## Related

- astral-sh/uv#23058 — Bash prompt values can change meaning under older compatibility modes (POSIX) (open; issue). Canonical match: reports the same uv 0.13.6 --prompt command substitution executing when activate is sourced, explicitly in both bash and zsh. The new reproduction changes only the shell and does not enable PROMPT_SUBST or render a prompt. Source confirms the shared unescaped prompt substitution.
- astral-sh/uv#9424 — Please comment on CVE-2024-53899 and virtualenv activation scripts (closed; issue). Historical activation-script command-injection discussion. Maintainer comments point to astral-sh/uv#8984 for environment-path quoting. Its single-quote path reproduction differs from this report's double-quoted prompt substitution; it does not establish that this prompt bug was previously fixed.
- astral-sh/uv#8984 — Allow apostrophe in venv name (merged; pull request). Merged November 15, 2024, fixing astral-sh/uv#8947. The diff escapes VIRTUAL_ENV_DIR using escape_posix_for_single_quotes and adds an apostrophe-path test; it leaves VIRTUAL_PROMPT replacement unchanged. Relevant historical fix, but not a fix for the reported prompt execution.

## Report decomposition

- **Source-time execution:** custom prompt text should remain literal when the activation script is sourced. The submitted example reports execution before any prompt redraw; the same behavior is described for bash and zsh in astral-sh/uv#23058.
- **Zsh prompt rendering:** the title and upstream reference concern re-expansion of PS1 with PROMPT_SUBST. The submitted example neither explicitly enables that option nor renders PS1, so it does not independently demonstrate that later execution stage.
- **Subsystem and triggers:** `uv venv --prompt`, `uv-virtualenv` activation-template substitution, the shared bash/zsh `bin/activate` script, shell command substitution in prompt text, and potentially zsh prompt expansion.
- **Exact leads:** CVE-2026-113726, CVE-2026-113723, PROMPT_SUBST, VIRTUAL_PROMPT, VIRTUAL_ENV_PROMPT, PS1, astral-sh/uv#23058, and upstream virtualenv commit `5b915367da82bd024b2a0d28d2f98007034b4aa2`. There is no reported error message. Fedora and uv 0.13.6 describe the reproduction environment, not a demonstrated platform-specific cause.

## Supporting evidence

The checkout examined is commit `39a02a0d9085f14dc3d5aa8655ac010a1fd377f8`.

- `crates/uv-virtualenv/src/virtualenv.rs:511` takes the prompt text and applies special encoding only for `activate.xsh`; the shared bash/zsh template receives the text unchanged. At line 536 the generator directly replaces the VIRTUAL_PROMPT placeholder.
- `crates/uv-virtualenv/src/activator/activate:106` inserts that placeholder inside the double-quoted condition operand and line 107 inserts it inside the double-quoted VIRTUAL_ENV_PROMPT assignment. Shell command substitution in the inserted text is therefore evaluated while sourcing. Both occurrences precede the VIRTUAL_ENV_DISABLE_PROMPT check.
- `crates/uv-virtualenv/src/activator/activate:119` through the PS1 assignment at line 121 adds VIRTUAL_ENV_PROMPT to PS1 without prompt-specific escaping. That later rendering concern must be distinguished from the source-time execution already established above.
- astral-sh/uv#23058 was opened on November 2, 2026, at 12:32 UTC, before this issue at 12:37 UTC. Its body explicitly describes both bash and zsh and uses the same prompt payload and uv version. Its sole retrieved comment adds another upstream fix lead; its timeline identifies this new report as a cross-reference, with no fixing PR shown.
- The upstream commit at https://github.com/pypa/virtualenv/commit/5b915367da82bd024b2a0d28d2f98007034b4aa2 extends existing prompt-redraw escaping to zsh, doubles percent characters, and adds tests that explicitly enable PROMPT_SUBST and render PS1. It is evidence about upstream virtualenv's later rendering stage, not a uv fix.
- The reviewed diff for astral-sh/uv#8984 changes VIRTUAL_ENV_DIR escaping while leaving VIRTUAL_PROMPT replacement unchanged. Its test activates an environment whose path contains an apostrophe. The fix was merged in November 2024, long before the reported uv 0.13.6 behavior.
- In astral-sh/uv#9424, a maintainer attributed the historical Unix path-quoting mitigation to astral-sh/uv#8984. Although that comment also mentions a batch fix, the retrieved metadata for astral-sh/uv#9466 shows it was closed without merging; it concerns Windows batch paths and is not a remedy for this report.

## Inspected alternatives

astral-sh/uv#8947 concerns activation syntax errors for an apostrophe-containing environment path and was fixed by astral-sh/uv#8984. It is historical context, not the same prompt-substitution bug.

astral-sh/uv#12662 concerns SCRIPT_PATH leaking into bash and zsh after activation. Its body and comments establish a shell-variable side effect, not command execution from prompt text.

astral-sh/uv#2668 concerns the custom prompt displaying incorrectly with Powerlevel10k. The reporter confirmed plain zsh worked and attributed the behavior to the theme, ruling it out as a match for command substitution.

## Search scope and limitations

Searched astral-sh/uv open and closed issues using authenticated gh: literal PROMPT_SUBST and both reported CVE identifiers; separate source-time execution and prompt-rendering searches using activate/commands, command substitution, command execution/venv, prompt/injection, prompt/substitution, zsh/PS1, escaping, quoting, and area:environments/special characters, without Fedora or version restrictions. Searched PRs across all states with gh search prs and gh pr list --state all for prompt, activate, and escaping; those searches returned no candidates. Followed issue comments and timelines to inspect historical PRs directly, including the merged astral-sh/uv#8984 diff and reviews. Read the reporter-linked upstream zsh fix. Ruled out astral-sh/uv#12662 (SCRIPT_PATH leakage), astral-sh/uv#2668 (Powerlevel10k display behavior), and astral-sh/uv#8947 (apostrophe-path syntax failure) as duplicates; astral-sh/uv#9466 was an unmerged Windows batch-path proposal. GitHub rate limiting blocked final supplemental exact-identifier, security-label, and PR searches, so search coverage is not exhaustive.

Broad searches that returned many unrelated results were followed by more specific symptom and mechanism searches. Historical candidate bodies, issue comments, PR reviews, the path-fix diff, and relevant cross-references were inspected. The missing PR keyword results were supplemented with direct retrieval of known historical PRs; they should not be interpreted as proof that no other PR exists.

## Validation and maintainer next step

This classification is based on source inspection and the matching existing report. No reporter-provided commands were executed, no builds or integration tests were run, and no independent zsh runtime reproduction was performed.

Centralize the report in astral-sh/uv#23058. Subsequent implementation work should validate source-time literal prompt handling separately from zsh rendering with PROMPT_SUBST enabled. The existing apostrophe-path regression test covers a different input boundary.

Only this issue-context README was updated. No checkout files or GitHub objects were changed.
