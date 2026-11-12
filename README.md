# add `uv activate` command to activate virtual env

Issue: astral-sh/uv#23227
Classification: duplicate

## Summary

The reporter requests a short, cross-platform `uv activate` command instead of typing `.venv\Scripts\activate` on Windows or `source .venv/bin/activate` on Unix. It would automatically activate the only virtual environment when one exists and offer a selection when several exist. No error, affected version, or regression is reported.

astral-sh/uv#1910 is the canonical activation-command discussion, reinforced by the exact cross-platform request in astral-sh/uv#14856 and the discovery duplicate astral-sh/uv#11835. The reporter's reference, astral-sh/uv#16993, proposes displaying instructions rather than activating the environment.

The reporter explicitly distinguishes the instruction-printing proposal in astral-sh/uv#16993 from the desired direct activation and asks whether direct activation is possible. In the issue discussion, maintainer woodruffw points to astral-sh/uv#1910 in particular for the technical constraints and complications of spawning a child shell.

## Draft response

This request is already tracked in astral-sh/uv#1910, with the same cross-platform use case discussed in astral-sh/uv#14856. Please add your suggestion about choosing among multiple environments to astral-sh/uv#1910 so the design discussion stays together. astral-sh/uv#16993 is narrower: it proposes displaying activation instructions. For now, you can use `uv run <command>` or your shell's activation script.

## Classification

The core request for a convenient, cross-platform virtual-environment activation command is already tracked by open astral-sh/uv#1910 and directly repeated in astral-sh/uv#14856. Maintainer handling of astral-sh/uv#11835 confirms that environment discovery belongs in the same discussion. Choosing among multiple environments is an additional design detail that can be discussed there; no previously fixed behavior or regression is reported.

The issue currently carries the enhancement label. The proposed triage classification is duplicate because the underlying new capability already has an open canonical discussion. The historical discovery request was closed as a duplicate, not fixed. No implementation or release claim is supported by the inspected evidence.

## Related

- astral-sh/uv#1910 (open issue), Add a command to activate the virtual environment, e.g., `uv shell`: Canonical discussion requesting a short command that enters the project's virtual environment without manually sourcing platform-specific activation scripts. It covers both current-shell activation and spawning a shell. Maintainers redirect equivalent activation and discovery requests here.
- astral-sh/uv#14856 (open issue), Add a command for `uv activate` for cross platform environment activation: Direct match for the requested command and Windows versus Unix activation friction. A maintainer explicitly identifies the overlap with astral-sh/uv#1910 and directs the author to that discussion; shell hooks are discussed as an approach to current-shell activation.
- astral-sh/uv#11835 (closed issue), Shortcut to activate uv-created venv  using uv's discovery mechanism: Requests discovering the environment and running the appropriate activation script, matching the automatic discovery aspect. A maintainer redirected it to astral-sh/uv#1910, and it was closed with the duplicate label, not resolved by an implementation. It does not specify an interactive choice among multiple environments.
- astral-sh/uv#16993 (open issue), `uv activate` could print an error message with the appropriate `source .venv/bin/activate` command: The reporter cites this in the live issue's edit. It proposes a helpful activation instruction when uv activate is attempted, rather than actually activating an environment. Maintainers distinguish a helpful error from adding a top-level command that only prints instructions.

## Supporting evidence

On astral-sh/uv#23227, maintainer woodruffw identifies the request as similar to astral-sh/uv#1910 and astral-sh/uv#16993, directing the reporter particularly to astral-sh/uv#1910 for discussion of the difficulty of activation without spawning a child shell and the complications of that approach. The supplied full discussion identifies woodruffw as a repository member. This directly supports the existing related-issue assessment; it does not announce an implementation, a formal duplicate closure, or a decision to reject the feature. Source: https://github.com/astral-sh/uv/issues/23227#issuecomment-6830710295

In astral-sh/uv#1910, the original proposal explicitly abstracts manual activation across operating systems. The discussion distinguishes entering a new shell from modifying the current shell. Maintainer zanieb explains that another process cannot source a script into the user's shell. This establishes a shell-integration constraint, rather than a defect in the existing activation scripts. The discussion remains open. Woodruffw specifically highlights contributor mitsuhiko's account of implementing and then removing similar functionality in Rye. Mitsuhiko describes an approach involving a pseudo-terminal connected to the parent terminal and injected keystrokes to manipulate the shell environment, reporting subtle breakage. This is implementation experience with that approach, not proof that every shell-integration design is infeasible. Highlighted source: https://github.com/astral-sh/uv/issues/1910#issuecomment-1963507697

In astral-sh/uv#14856, the report lists Git Bash, CMD, PowerShell, and Unix activation commands and requests `uv activate`. Maintainer zanieb calls it a likely duplicate of astral-sh/uv#1910 and recommends reading that discussion. The author proposes shell hooks to perform activation in the current shell; this remains a proposal.

In astral-sh/uv#11835, the report requests using uv's discovery mechanism to find and run the appropriate activation script, including outside a project with a pyproject.toml. Maintainer zanieb redirects it to astral-sh/uv#1910, and the issue is closed with the duplicate label. This is direct precedent for centralizing the discovery aspect. None of these inspected discussions establishes support for an interactive multiple-environment selector.

In astral-sh/uv#16993, the requested behavior is to provide activation instructions even if actual activation cannot be supported. Maintainer charliermarsh clarifies that this could be a more helpful error message rather than a new command. Maintainer zanieb links astral-sh/uv#14038, which requests printing the shell-specific activation command. These are adjacent usability proposals, not implementations of the requested activation command.

The checkout's docs/pip/environments.md documents shell-specific activation scripts and automatic environment discovery for uv commands. Maintainer konstin also confirms in astral-sh/uv#16993 that `uv run` avoids manual activation while sourcing the activation script remains supported. These are current alternatives; neither provides the requested interactive selector.

## Search scope and exclusions

The report was decomposed before searching into:

- A short `uv activate` command that performs activation.
- One command across operating systems and shells.
- Automatic discovery when exactly one virtual environment exists.
- A choice among multiple virtual environments.

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests with authenticated gh. Literal searches covered uv activate, activate in titles, source .venv/bin/activate, and the Windows activation path. Separate conceptual searches covered uv shell, shell activation and hooks, environment discovery, multiple environments, selection, choosing environments, switching, and interactive activation. Implementation and fix searches covered activation, venv discovery, and shell-hook PRs; no implementing PR was found. Inspected candidate bodies and comments, followed duplicate referrals to astral-sh/uv#1910, and checked its PR cross-references. Inspected but ruled out astral-sh/uv#7744 and astral-sh/uv#4245 as automatic activation on directory changes, astral-sh/uv#11472 as script execution environment selection, and astral-sh/uv#13542 as uv sync documentation. Followed astral-sh/uv#16993 to astral-sh/uv#14038, which only requests printing activation commands.

The automatic-directory-activation issues require a different trigger from an explicit activation command. The script-selection issue concerns the environment used by `uv run --script`, not entering an interactive environment. The referenced documentation PR explains synchronizing an existing project and does not implement activation. The printing-only proposals do not supersede astral-sh/uv#1910 as the canonical discussion for this report.

## Handoff

The maintainer reply already directs the reporter to astral-sh/uv#1910, so that referral does not need to be repeated. Retain the duplicate recommendation; choosing among multiple environments remains a design detail for the canonical discussion. The earlier draft response above is retained as existing triage context. No additional public response is proposed here. This handoff and proposed classification have not been posted to GitHub. No checkout files were modified for this triage.
