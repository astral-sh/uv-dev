# Scrub home directory path from `uv` output

Issue: astral-sh/uv#22986

Classification: enhancement

## Summary

The reporter wants uv to replace the current user's home-directory prefix with `~` in output by default, reducing personal information exposed when verbose logs are shared in bug reports. An opt-out flag is suggested, not required. The motivating command is `uv audit --verbose`, but the requested capability applies broadly to uv output.

The concrete examples are `DEBUG Found user configuration in: /home/jdoe/.config/uv/uv.toml`, which should display `~/.config/uv/uv.toml`, and `DEBUG Found workspace root: /var/home/jdoe/rust-lang/fls`, which should display `~/rust-lang/fls`. The latter describes an atomic-distribution setup where the home directory has both symlinked and resolved spellings. Supporting both spellings is part of the request.

No duplicate found. astral-sh/uv#4345 and merged astral-sh/uv#5311 and astral-sh/uv#5313 cover narrower managed-interpreter path omission while retaining full paths in verbose logs.

## Maintainer position

In astral-sh/uv#22986, zanieb (identified as a repository member in the supplied discussion) questions whether home-path scrubbing is worthwhile: verbose logs can contain other personal information and are not designed to be shared in a de-identified form. Source: the October 29, 2026 comment at 16:47:04 UTC, comment ID 6502813576.

zanieb also questions whether less-specific paths improve the user experience, because removing personal information can obscure what is happening in diagnostic logs. They see potential value in a general mode that produces stripped logs for sharing, while describing that as a much larger project. Source: the October 29, 2026 comment at 18:36:32 UTC, comment ID 6505216618.

The maintainer's concerns therefore include diagnostic clarity as well as the value and privacy expectations of the feature. A general stripped-log mode is a suggested direction, with no approved design or implementation commitment in the supplied discussion. There is no explicit final acceptance or rejection of this issue. Home-prefix abbreviation would address the reporter's examples, but should not be presented as making verbose logs de-identified. The enhancement classification and existing source findings remain applicable.

## Reporter impact and priority

The reporter says they limited the transcript in astral-sh/uv#22985 to `-v` because reviewing the additional information in `-vv` or `-vvv` output for sharing implications would take too much time. This is a reported barrier to supplying more detailed diagnostics; it does not establish what sensitive information the omitted logs contained. They describe the enhancement as low priority and would like the issue kept open for a potential community contribution. That preference is not maintainer approval to implement it.

The reporter would also accept the maintainer's proposed general stripped-log mode; home-prefix abbreviation was intended as a smaller initial proposal. They favor redaction by default with a way to obtain unredacted details when needed for diagnosis. This clarifies the reporter's acceptable scope and preferences, without establishing an agreed design or maintainer approval.

The reporter cites astral-sh/uv#9452 as encouraging log sharing. Inspection confirms that its “What information to include” section requests command output with `--verbose` and suggests repeated `-v` flags for additional logs. The inspected issue body contains no warning about personal or sensitive information in those logs. The reporter interprets that encouragement as implying some degree of safety; the maintainer's stated position remains that verbose logs are not designed for de-identified sharing.

## Classification

The report requests a new default privacy-oriented display policy: abbreviate home-directory prefixes, recognize symlinked and resolved home paths, and possibly provide an opt-out. Source inspection confirms that the reported debug messages currently render paths without home abbreviation. Earlier managed-Python changes deliberately retained paths in debug logs, so they do not establish a regression or duplicate; no incorrect path value or broken existing scrubbing behavior is established.

The source evidence establishes the absence of home abbreviation at the reported logging sites. The symlink layout is reporter-provided context; it was not independently reproduced. The linked audit report identifies uv 0.13.6 on Fedora 44 Kinoite, but this issue does not claim a version regression.

## Related

- astral-sh/uv#4345 — Elide interpreter path from debug logs when using managed toolchain (issue, closed). Closest path-elision precedent. Despite its title, the closing discussion demonstrates omission of managed-interpreter paths from routine output while verbose logs still contain full paths. It does not cover home-prefix replacement across configuration, workspace, and other diagnostics.
- astral-sh/uv#5311 — Omit interpreter path during `uv venv` with managed Python (pull request, merged). Implemented the narrower managed-Python display change for uv venv. A maintainer's verbose-output example explicitly retains absolute home paths in DEBUG messages; this is adjacent precedent, not a home-directory scrubbing fix.
- astral-sh/uv#5313 — Omit interpreter path from output when using managed Python (pull request, merged). Extended astral-sh/uv#5311 to project commands. The maintainer explicitly states that interpreter paths should remain in DEBUG logs, distinguishing that change from the requested default home-path abbreviation.

- astral-sh/uv#9452 — Before posting in the issue tracker (issue, open). Reporting guidance explicitly requests `--verbose` output and describes repeated `-v` flags, without a privacy warning in the inspected body. This supports the reporter's concern about sharing expectations and the effort needed to review logs; it is guidance context, not a duplicate feature request.

## Supporting evidence

Source inspected at checkout commit `c76ff10bab78a44da9cd26f81d15c1e5ad018e03`:

- `crates/uv-settings/src/lib.rs:59`: user configuration discovery constructs the configuration path; the searching and found debug messages at lines 66 and 69 use `file.display()` directly.
- `crates/uv-workspace/src/workspace.rs:554` and line 1880: workspace-root debug messages use `workspace_root.simplified_display()`.
- `crates/uv-fs/src/path.rs:55`: `simplified_display()` uses `dunce::simplified(...).display()`, which handles Windows path prefixes without substituting the user's home directory. `user_display()` at line 63 can strip the current working directory, but contains no home-prefix abbreviation.
- `crates/uv-logging/src/lib.rs:63`: field formatting strips ANSI escape sequences and writes the formatted value; it does not scrub home paths.

Historical discussion provides the decisive scope distinction:

- astral-sh/uv#5311 merged on July 23, 2024. The maintainer's verbose example retains absolute home paths in managed-Python discovery messages, even though the routine `Using Python ...` line omits the interpreter path.
- astral-sh/uv#5313 merged on July 24, 2024. Its maintainer comment explicitly says, “Note we should still show the interpreter path in DEBUG logs.”
- astral-sh/uv#4345 was closed on May 18, 2026 with examples comparing uv 0.2.37 and 0.3.0. Those examples still contain home-directory paths in verbose output. The closing comment also points to astral-sh/uv#6166, merged August 19, 2024, which enabled preview behavior by default for uv 0.3.0; it did not add home-path redaction.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests with authenticated gh. Separately searched home-path scrubbing, default/optional redaction, and symlink/canonical-home handling. Literal searches included the two debug messages, home directory/output, scrub, redact, and tilde; conceptual searches included privacy, personal information, anonymization, sanitization, abbreviation, elision, and area:cli/area:tracing vocabulary. Fix-oriented searches covered merged home/path changes and closed redaction work; followed astral-sh/uv#4345 through astral-sh/uv#5311, astral-sh/uv#5313, and astral-sh/uv#6166, inspecting comments and release context. Ruled out astral-sh/uv#16288 (tilde expansion in configuration inputs), astral-sh/uv#5308 (displaying interpreter symlink targets), astral-sh/uv#5440 and astral-sh/uv#5446 (Windows executable canonicalization), and astral-sh/uv#6840 (root-directory path rendering). The reporter-linked astral-sh/uv#22985 concerns advisory duplication and fix versions, not path scrubbing. A transient search rate limit was resolved before the final targeted searches. No matching duplicate or historical home-scrubbing fix was found.

The closest-looking excluded issues were inspected with their comments:

- astral-sh/uv#16288 requests expanding `~` when reading configuration values. Its discussion concerns portable cache configuration and filesystem placement; this is input interpretation rather than output abbreviation.
- astral-sh/uv#5308 requests displaying interpreter symlink targets in `uv python list`. Maintainer comments discuss `read_link` and a possible canonical-paths option to clarify interpreter identity, not hiding home-directory prefixes.
- astral-sh/uv#5440 and its merged fix astral-sh/uv#5446 address Windows UNC executable canonicalization causing virtual-environment failures. The fix avoids canonicalizing executables on Windows; it does not implement privacy-oriented display formatting.
- astral-sh/uv#6840 concerns retaining the leading slash when displaying paths from the filesystem root. It has no comments and does not request home abbreviation.
- astral-sh/uv#22985 supplied the motivating verbose transcript, already manually scrubbed by the reporter. Its subject is duplicate vulnerability advisories and incorrect fix versions, so it is not a canonical discussion for this enhancement.

## Maintainer next step

Resolve the intended direction before implementation: whether to pursue home-prefix abbreviation in ordinary output or investigate the broader stripped-log mode suggested by the maintainer. Evaluate the reporter's barrier to sharing detailed logs alongside the maintainer's concern that removing path detail can obscure diagnostics. The general mode would require a broader scope than replacing home prefixes; no design, acceptance, or implementation commitment is established. Separately, consider whether the log-sharing guidance in astral-sh/uv#9452 should clarify expectations; no documentation change has been agreed in the supplied discussion.

If maintainers agree to pursue it, settle which human-readable diagnostics should use abbreviation, whether path-valued command output needs separate treatment, how both the configured home path and its resolved target should be recognized, and whether users need an opt-out. Any implementation should account for both direct path formatting and existing shared display helpers; changing only `user_display()` would not affect the two reported logging sites.

This handoff is based on read-only GitHub and source inspection. No reproduction, build, or tests were run. Only this temporary README was updated; no checkout files or GitHub objects were changed.
