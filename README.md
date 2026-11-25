# Bash activator leaves the --prompt value unescaped in PS1 when promptvars is off, so prompt expansion later executes it

Issue: astral-sh/uv#23376

Classification: duplicate

## Summary

The reporter used uv 0.13.7 on Fedora 45 and expected `uv venv --prompt` to set literal prompt text. They observed shell evaluation during a session that sourced `bin/activate` with Bash's `promptvars` disabled and later enabled it. The report also raises switching to POSIX mode and inheriting the exported `PS1` in an interactive child shell. The existing Bash issue, astral-sh/uv#23058, tracks this underlying failure to keep prompt text literal. Open Bash work distinguishes evaluation while sourcing the script from evaluation while displaying the prompt.

## Draft response

Thanks for reporting this. I'm marking this as a duplicate of astral-sh/uv#23058, where we're tracking Bash activation prompts being interpreted as shell syntax. In uv 0.13.7, the explicit prompt can be evaluated as soon as `bin/activate` is sourced, even with `promptvars` off, so the reported result doesn't show that evaluation waited for the setting to change. The activator also puts the prompt text directly into `PS1` and exports it. Let's track changes to prompt expansion after activation and inherited prompts in astral-sh/uv#23058 so the fix can be checked at both stages.

## Classification

Both reports concern the same generated Bash activator treating a value passed to `uv venv --prompt` as shell syntax rather than display text. The supplied reproduction can hit the source-time behavior already reported in astral-sh/uv#23058; it does not isolate a second, later evaluation. The shell-state transition is worth covering with the same fix, not evidence of a separate uv regression. The relevant source is unchanged between the 0.13.7 release on November 3, 2026 and the November 23, 2026 checkout, and no merged uv fix for this behavior was found. The regression identified in GHSA-cx6f-vjhg-g3cc (CVE-2026-115586) is in upstream `virtualenv`, not an established uv regression.

## Related

- astral-sh/uv#23058 (open): Canonical Bash issue. Its body reports source-time evaluation of a `uv venv --prompt` value in the shared activator, and linked work also covers Bash prompt redraw and POSIX mode.
- astral-sh/uv-dev#3068 (open): Proposed Bash fix for astral-sh/uv#23058 covering both source-time quoting and prompt rendering. It explicitly considers POSIX mode with `promptvars` disabled; behavior when expansion changes after activation or `PS1` is inherited still needs verification.
- astral-sh/uv-dev#3065 (open): Draft Bash test PR linked to astral-sh/uv#23058 and astral-sh/uv-dev#3068. It covers source and redraw behavior and investigates `promptvars` and POSIX mode; it is not a shipped fix.
- astral-sh/uv#23155 (open): Public PR linked to astral-sh/uv#23058. It quotes explicit prompts in generated scripts and tests source-time evaluation, but does not change `PS1` handling.
- astral-sh/uv#23059 (open): Related zsh report that links to the same source-time issue. Its prompt-redraw path is specific to zsh's `PROMPT_SUBST`, not Bash's `promptvars` or POSIX mode.

## Evidence

- `crates/uv-virtualenv/src/virtualenv.rs:514` leaves prompt text unescaped for the shared Bash activator, and `crates/uv-virtualenv/src/virtualenv.rs:539` inserts it into the template. The tagged 0.13.7 source uses the same rendering logic.
- `crates/uv-virtualenv/src/activator/activate:106` places the explicit value in double-quoted shell source, where it can be evaluated while sourcing, independently of Bash prompt settings. `crates/uv-virtualenv/src/activator/activate:119` separately inserts the resulting value into `PS1` and exports it without a `promptvars` or POSIX-mode check. The 0.13.7 activator is byte-identical to the checkout's copy.
- These checks establish the two code paths, but do not independently confirm which Bash prompt-state transitions reproduce later evaluation. No reported commands were run.

## Proposed Fix Scope

A contributor proposes escaping the Bash prompt prefix when assigning `PS1`, taking the upstream virtualenv fix for GHSA-cx6f-vjhg-g3cc as a reference. They plan regression coverage for re-enabling `promptvars` after activation and for an interactive child Bash inheriting `PS1`. The comment provides no patch or test results and does not record a maintainer decision.

Review this proposal alongside astral-sh/uv-dev#3068 under astral-sh/uv#23058. Changing only `PS1` would leave the earlier source-time evaluation of explicit prompts untouched. Regression coverage should distinguish literal handling while sourcing `bin/activate` from literal rendering after prompt expansion changes or in a child shell.

## Search Scope

Used authenticated GitHub CLI searches across open and closed issues and open, closed, and merged PRs in astral-sh/uv. Literal searches covered `promptvars`, `PS1`, `--prompt`, `VIRTUAL_ENV_PROMPT`, the advisory ID, and the CVE. Separate symptom and cause searches covered Bash prompt expansion, child-shell inheritance, command substitution, `PROMPT_SUBST`, and activation-script escaping and quoting. Followed candidate comments and timeline references into astral-sh/uv-dev and checked merged and closed PRs for a released fix. Ruled out astral-sh/uv#23246 as a Bash fix (zsh only), astral-sh/uv#23117 as a shipped fix (closed unmerged), astral-sh/uv#2668 and astral-sh/uv#13456 as prompt-display or formatting reports, and astral-sh/uv#12662 as an unrelated activation variable leak.
