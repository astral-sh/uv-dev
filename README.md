# Can --default instead add its path to the system PATH?

Issue: astral-sh/uv#21986

Classification: enhancement

## Summary

The report asks for `uv python install <version> --default` to put uv's Python executable directory on the Windows system PATH so that uv's `python.exe` and `python3.exe` take precedence over the App Installer aliases in `%LOCALAPPDATA%\Microsoft\WindowsApps`.

The current command has narrower behavior than the report assumes: `--default` controls which executables uv installs in its executable directory, adding the unversioned `python` and `python3` names alongside the versioned name. It does not itself update PATH. The separate `uv python update-shell` command handles PATH updates; on Windows, the current implementation prepends the executable directory to `HKEY_CURRENT_USER\Environment\PATH`. There is no current support for writing the machine-wide PATH.

The closest prior report is astral-sh/uv#15581, where `--default` created the expected executables but WindowsApps still won on one machine because of PATH order. The reporter corrected the ordering manually, and a maintainer said uv could not fix that PATH. The implementation pull request, astral-sh/uv#8650, also identified detecting or warning when the installed executable is not at the front of PATH as future work. No existing issue or pull request was found that tracks writing uv's executable directory to the Windows system PATH, so this is a new enhancement rather than a duplicate.

## Draft response

Thanks. `--default` currently only adds the unversioned `python.exe` and `python3.exe` launchers to uv's Python executable directory; it does not update PATH itself. On Windows, `uv python update-shell` prepends that directory to the current user's PATH in `HKEY_CURRENT_USER\Environment\PATH`.

Writing the machine-wide PATH would be new behavior, would require elevation, and would affect every user on the machine, so it is not a direct replacement for the current per-user update. The same WindowsApps ordering problem was reported in astral-sh/uv#15581 and was handled by correcting PATH order manually. To confirm whether this case differs, could you share the output of `uv python dir --bin` and `where.exe python` from a newly opened shell, plus whether the WindowsApps entry is in the user or system PATH? That will show whether the per-user entry was applied but is being outranked, or whether the shell has not picked up the update.

## Classification

This is an enhancement. The requested system-wide PATH mutation is not part of `--default`'s documented or implemented behavior: the flag installs additional executable names, while PATH configuration belongs to `uv python update-shell`, whose Windows implementation intentionally operates on the current-user registry environment. The report does not establish a regression or a failure of an existing system-PATH feature.

The claim that the App Installer alias outranks uv is plausible, but the exact PATH composition is not provided. In particular, both entries may be user-level on a typical Windows installation, while astral-sh/uv#15581 showed a machine-specific ordering that needed manual correction. The requested system-PATH support remains new functionality regardless of which PATH layer contains WindowsApps.

## Related

- astral-sh/uv#15581 — Closed issue, “Windows uv python install should also provide python and python3 binaries (symlinks).” This is the closest observed behavior: `--default` installed the unversioned executables, but WindowsApps still resolved first on one machine because of PATH ordering. The reporter fixed the order manually, and the maintainer characterized PATH correction as outside uv's control. It does not track a machine-wide PATH option.
- astral-sh/uv#7710 — Closed issue, “uv python install at fixed location?” This requested a stable, authoritative `python` at the front of PATH. Maintainers noted that uv could not guarantee the first PATH entry; the issue was later considered addressed by `--default`. It is conceptually related but predates and does not cover Windows system-PATH modification.
- astral-sh/uv#8650 — Merged pull request, “Add `uv python install --default`.” This defines `--default` as installing `python` and `python3` executables in addition to the versioned executable. Its description leaves a warning for executables not at the front of PATH as future work; it does not add or propose machine-wide PATH mutation.

## Search evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal searches included `--default` with `system PATH`, `user PATH`, `WindowsApps`, `app execution alias`, `DesktopAppInstaller`, `python install`, `python.exe`, `HKEY_CURRENT_USER`, and `update-shell`. Conceptual searches used `machine PATH`, system environment, administrator/elevation, PATH order/precedence, executable directory, Microsoft Store shim, default/global Python, stable shim, and front/tip of PATH. Fix-oriented searches included the `python-install-default` feature, Windows shell PATH updates, registry PATH handling, and merged work implementing `--default`.

astral-sh/uv#15237 was inspected because it says `--default` does not make a Python version global, but it concerns prerelease executable selection on macOS rather than Windows PATH layers. astral-sh/uv#6348 and astral-sh/uv#2264 were inspected because they involve Windows App Execution Aliases or Microsoft Store Python, but they concern uv's interpreter discovery and process spawning rather than shell resolution of uv-installed executables. No exact existing tracker for writing the system PATH was found.
