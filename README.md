# Error with venv activation command for ksh on OpenBSD

Issue: astral-sh/uv#22537

Classification: bug

## Summary

On OpenBSD, the reporter's source-built uv 0.13.0 successfully creates a virtual environment with `uv venv`, then prints `Activate with: source .venv/bin/activate`. OpenBSD's default ksh rejects that command with `ksh: source: not found`. The requested correction is to print `. .venv/bin/activate`.

The checkout confirms that `Shell::Ksh` shares the Bash/Zsh branch that emits `source`. astral-sh/uv#5640 introduced this branch. The closest symptom match, astral-sh/uv#18773, concerns a different, confirmed shell-detection mismatch. No matching duplicate or prior fix was found.

## Report scope

- Symptom: the activation command printed after successful environment creation cannot be invoked in the reported shell.
- Expected behavior: print the dot command for ksh, retaining the appropriate activation-script path and shell quoting.
- Command and subsystem: `uv venv` activation-hint rendering, using `uv-shell` shell identification.
- Trigger: OpenBSD ksh, reported as `@(#)PD KSH v5.2.14 99/07/13.2`, with uv 0.13.0 for `x86_64-unknown-openbsd`. The build used Rust 1.98.1 and the selected interpreter was CPython 3.14.8 at `/usr/local/bin/python`.
- Exact identifiers: `Shell::Ksh`, `KSH_VERSION`, `source .venv/bin/activate`, and `ksh: source: not found`.

The report concerns the printed invocation. It does not establish a failure to create the environment or a failure inside the activation script.

## Draft response

You're right: uv already recognizes ksh, but the activation hint groups it with Bash and Zsh and prints `source`. Use `. .venv/bin/activate` in place of that command on OpenBSD ksh. The fix should give `Shell::Ksh` a separate `.` branch while retaining path quoting, with a regression test for the printed hint. This differs from astral-sh/uv#18773, which involved detecting the wrong shell.

## Classification

The source explicitly prints source for Shell::Ksh, although OpenBSD ksh requires the dot command. This is incorrect user-facing guidance for an already recognized shell. The closest issue concerns a different shell-detection mechanism, and the historical ksh PR introduced this branch rather than fixing it; the evidence does not establish a recurrence of a previously fixed bug.

astral-sh/uv#18773 was closed on April 1, 2026 after the reporter confirmed that correcting the login-shell value removed the misleading suggestion. Its discussion does not establish a fix for correctly detected ksh. astral-sh/uv#5640 merged on July 31, 2024 and already added ksh to the `source` branch. This predates the reported uv 0.13.0 build and supports treating this as an existing correctness defect, rather than claiming a recent regression.

## Related

- astral-sh/uv#5640 — fix(venv): make relocatable activation scripts support ksh (pull request, merged). Introduced Shell::Ksh detection and added it to the Bash/Zsh branch that prints source. The July 2024 diff directly explains the current hint. Its fix concerned script-path discovery for relocatable environments, not OpenBSD ksh's missing source command; it is historical context, not a duplicate or a prior fix for this defect.
- astral-sh/uv#18773 — `uv venv` recommends use of `source` in incompatible shells (issue, closed). Reports the same unusable source recommendation in dash/ash. Maintainer discussion and reporter confirmation identified SHELL pointing to a different login shell; correcting SHELL removed the suggestion. Here, even correctly detected Shell::Ksh explicitly produces source, so the underlying defect differs.

## Supporting evidence

- At checkout `a79aac95dc8f1d79e4396fdcb262d22c520cc1f6`, [activation command selection](https://github.com/astral-sh/uv/blob/a79aac95dc8f1d79e4396fdcb262d22c520cc1f6/crates/uv/src/commands/venv.rs#L389-L411) explicitly matches `Shell::Bash | Shell::Zsh | Shell::Ksh` and formats `source {}` using `shlex_posix`.
- [Shell detection](https://github.com/astral-sh/uv/blob/a79aac95dc8f1d79e4396fdcb262d22c520cc1f6/crates/uv-shell/src/lib.rs#L57-L71) recognizes an exported `KSH_VERSION`; the [shell-name parser](https://github.com/astral-sh/uv/blob/a79aac95dc8f1d79e4396fdcb262d22c520cc1f6/crates/uv-shell/src/lib.rs#L311-L320) also recognizes `ksh`. Correct detection therefore does not eliminate this incorrect hint.
- The diff and maintainer discussion in astral-sh/uv#5640 show both the addition of ksh to the shell enum and the addition of ksh to the `source` rendering branch. The PR's principal script change addressed locating a sourced script for `--relocatable`.
- In astral-sh/uv#18773, maintainer comments identify the login-shell versus current-shell distinction, and the reporter confirms that changing `SHELL` removes the suggestion. This is the key reason not to centralize the current report there.
- The earlier fish-specific activation-hint issue astral-sh/uv#2174 led to astral-sh/uv#2221, merged March 5, 2024. That diff introduced shell-specific hints but did not yet include ksh. It supplies background rather than a fix for this report.

Source inspection establishes the incorrect rendering branch. No OpenBSD runtime reproduction or build was performed; full activation-script compatibility was not independently verified.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered ksh, OpenBSD activation, KSH_VERSION, Shell::Ksh, and source: not found; conceptual searches covered Korn, POSIX, dot, activation commands, shell detection, and environment hints without platform/version constraints. PR searches covered ksh, activation, activate, venv source, POSIX, and shell detection; keyword searches returned no PR matches, so issue timelines and source-file commit history were also inspected. Read candidate comments and the diffs of astral-sh/uv#5640 and astral-sh/uv#2221. Ruled out astral-sh/uv#18773 as a duplicate because its confirmed mechanism was shell misidentification. Also ruled out astral-sh/uv#7480 and astral-sh/uv#10498, which fail inside activation scripts, and the fish-specific historical fix tracked by astral-sh/uv#2174 and astral-sh/uv#2221. No prior fix or existing tracker for the specific ksh command-rendering defect was found.

The open issue astral-sh/uv#7480 concerns yash rejecting zsh-specific syntax inside the activation script after it is sourced. The open issue astral-sh/uv#10498 concerns an unset `OSTYPE` under `set -u` on Alpine. Both were inspected, including comments, and excluded because neither tracks an unavailable `source` command in the printed hint.

PR keyword searches returned no entries despite historical PRs being retrievable directly. Issue timelines and the `uv-shell` file history supplied the historical PRs reviewed here; the absence of another tracker is a search finding, not proof that none exists.

## Suggested next step

Handle `Shell::Ksh` separately in activation-hint rendering and emit `.` while retaining `shlex_posix` path quoting. Add focused regression coverage in the existing venv test module after checking nearby tests and their feature gates. Validate the printed command on OpenBSD ksh.

This handoff contains a proposed reply for maintainer review. No issue comment, label, pull request, or other GitHub change was made, and no checkout files were modified.
