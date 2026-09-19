# `install.sh` unresolved `$XDG_DATA_HOME/../bin` causes false "shadowed by other commands" warning and unnecessary rc-file edits

Issue: astral-sh/uv#21837

Classification: duplicate

## Summary

With `XDG_DATA_HOME=$HOME/.local/share` and `$HOME/.local/bin` already on `PATH`, the standalone installer selects the lexically unresolved install directory `$HOME/.local/share/../bin`. The report describes two consequences: the installer does not recognize that the equivalent directory is already on `PATH`, so it writes shell startup files and creates fish configuration directories; then its shadow check compares `command -v` output such as `$HOME/.local/bin/uv` with `$HOME/.local/share/../bin/uv` and incorrectly warns that `uv` and `uvx` are shadowed.

The current generated installer confirms these lexical comparisons. It assigns `_install_dir="$XDG_DATA_HOME/../bin"`, checks `PATH` for that exact string before deciding whether to edit profiles, and compares `command -v` output directly with `$_install_dir/$_bin_name`.

Open astral-sh/uv#13225 already tracks the same underlying path-equivalence failure. Its comments use the same `XDG_DATA_HOME=$HOME/.local/share` setup, explain that `$HOME/.local/bin` is already on `PATH` but does not match `$HOME/.local/share/../bin`, and report the resulting shell-file edits. A maintainer explicitly concludes that path detection should handle this case more robustly. The new report adds a precise reproduction of the false shadow-warning consequence, but it does not require a separate canonical discussion.

## Draft response

Thanks for the detailed reproduction. This is the same underlying path-detection problem already tracked in astral-sh/uv#13225: when `XDG_DATA_HOME` is set to `$HOME/.local/share`, the installer keeps `$HOME/.local/share/../bin` as a lexical path and does not recognize the equivalent `$HOME/.local/bin` entry already on `PATH`. The current installer uses that same unresolved value for both the `PATH`-presence check and the shadow check, which accounts for the unnecessary profile edits and the false warning you observed.

Let's centralize the fix in astral-sh/uv#13225. I'll close this as a duplicate, while retaining this report as a useful concrete reproduction of the shadow-warning side effect.

## Classification

This is a `duplicate`. The behavior is an established correctness problem: the installer treats equivalent filesystem paths as different, leading to both misleading user-facing output and unnecessary configuration changes. Duplicate takes precedence because open astral-sh/uv#13225 already contains the same trigger, path mismatch, actual behavior, and requested improvement to installer path detection.

This is not a regression of closed astral-sh/uv#12101. That issue's confirmed upstream fix changed the shadow check so an empty `command -v` result would not be reported as shadowing. Here, `command -v` returns a non-empty resolved path, but the direct string comparison rejects it because the configured install path still contains `..`.

The lower-priority request to avoid creating fish configuration on systems without fish, and the broader preference for less invasive profile editing, are adjacent installer-policy concerns. They do not change the classification of the primary actionable report, and astral-sh/uv#13225 already discusses the broader profile-edit behavior.

## Related

- astral-sh/uv#13225 — Open issue and canonical match. Its comments report `XDG_DATA_HOME=$HOME/.local/share` producing `$XDG_DATA_HOME/../bin`, failing to recognize `$HOME/.local/bin` on `PATH`, and editing shell files; a maintainer says this exact path-detection case needs more robust handling.
- astral-sh/uv#8420 — Merged pull request that introduced the XDG installer fallback. Its review explicitly observed that `XDG_DATA_HOME` yields an unresolved `/../bin` path and identified normalization as fixable, establishing the historical origin of the mismatch.
- astral-sh/uv#12101 — Closed issue with the same false shadow-warning symptom but a different confirmed mechanism. Its fix added a non-empty guard for `command -v`; astral-sh/uv#21837 passes that guard with a non-empty but lexically different equivalent path.

## Search coverage and exclusions

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries included the exact warning, `XDG_DATA_HOME/../bin`, `share/../bin`, `NO_MODIFY_PATH`, and `XDG_BIN_HOME`. Conceptual queries covered installer shadow detection, equivalent or normalized paths, shell-profile and dotfile changes, fish `conf.d` creation, and installer path defaults. Strong candidates were inspected with their comments and linked discussions.

astral-sh/uv#10413 is an opt-out support question rather than the path-equivalence defect. astral-sh/uv#13859 concerns genuine shadowing caused by third-party installers, astral-sh/uv#14849 concerns VS Code/Snap XDG isolation, and astral-sh/uv#15345 confirmed an older binary actually preceding the new installation on `PATH`.
