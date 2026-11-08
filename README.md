# activate.bat sets wrong VIRTUAL_ENV when the path contains an apostrophe

Issue: astral-sh/uv#23149

Classification: bug

## Summary

The reporter creates a virtual environment with uv 0.13.7 (316c827e0, November 3, 2026), using CPython 3.15.2 on Windows 11 x86_64, under C:\Tools\it's a test. Running .venv\Scripts\activate.bat sets VIRTUAL_ENV to C:\Tools\it'"'"'s a test\.venv instead of the actual environment directory. The report says activation fails.

Current source confirms that an ordinary, non-relocatable batch activator receives a path processed by the POSIX single-quote escaping helper. The same VIRTUAL_ENV value is used to construct PATH, explaining why the wrong path affects interpreter lookup. This conclusion comes from source inspection; the Windows command sequence was not executed in this Linux environment.

astral-sh/uv#8947 and merged astral-sh/uv#8984 explain the historical POSIX apostrophe fix and the fallback now corrupting batch paths. Closed, unmerged astral-sh/uv#9466 proposed separate batch handling but did not ship a fix.

## Report decomposition

- Command and subsystem: uv venv generates .venv/Scripts/activate.bat; cmd.exe executes the generated script.
- Trigger: an apostrophe in any component of the full environment path. The example also contains spaces; the observed inserted characters specifically match apostrophe escaping.
- Observable symptom: VIRTUAL_ENV contains the POSIX sequence '"'"', so it no longer names the created environment. The reporter observes failed activation.
- Expected behavior: VIRTUAL_ENV names the actual environment directory, and PATH selects that environment's Scripts directory.
- Exact search identifiers: activate.bat, VIRTUAL_ENV, apostrophe, single quote, escape_posix_for_single_quotes.
- There is no separate feature request or Python-version-specific failure established by this report.

## Draft response

Your example matches the current source: non-relocatable activate.bat paths go through POSIX apostrophe escaping before being inserted into the batch script. astral-sh/uv#8984 added this for POSIX activation, and the existing apostrophe test only runs on Linux. The fix needs batch-specific path handling and a Windows activation regression test. For now, create the environment at a path with no apostrophes in any directory component.

## Classification

The current source applies POSIX apostrophe escaping to ordinary activate.bat paths and then uses that value for VIRTUAL_ENV and PATH, incorrectly changing a valid Windows path. This is a Windows correctness bug introduced by the broad fallback in astral-sh/uv#8984, rather than a recurrence of the POSIX-shell failure fixed for astral-sh/uv#8947. The related batch proposal astral-sh/uv#9466 was never merged and addressed double quotes. No existing open tracker for the same Windows apostrophe failure was found.

The historical POSIX issue is useful context but is already fixed for its original shell. Its fix introduced the shared escaping fallback responsible for the Windows behavior. There is no evidence that a working, previously shipped batch-specific apostrophe fix has subsequently been reverted. The unmerged double-quote proposal does not establish that this Windows apostrophe failure was fixed.

## Related

- astral-sh/uv#8947 (closed; issue): Uv fails to activate environment in folder with apostrophe. Historical apostrophe-path activation bug in uv 0.4.25 on Fedora, using source .venv/bin/activate. Maintainer comments discuss POSIX quoting; astral-sh/uv#8984 closed it. The new report concerns cmd.exe receiving POSIX escape syntax, so it is a Windows bug arising from the historical fix, rather than the original POSIX failure.
- astral-sh/uv#8984 (merged; pull request): Allow apostrophe in venv name. Merged November 15, 2024 and released in uv 0.5.3 to fix astral-sh/uv#8947. Its diff applies escape_posix_for_single_quotes to the default activation-template branch, including non-relocatable activate.bat, and adds a Linux-only apostrophe test. That fallback remains in the checkout and explains the reported batch-path corruption.
- astral-sh/uv#9466 (closed; pull request): Escape path in batch script. Proposed a dedicated non-relocatable activate.bat path branch that bypassed POSIX escaping, but focused on doubling double quotes during the astral-sh/uv#9424 discussion. Closed without merging after the author noted that Windows filenames cannot contain double quotes. This is a relevant abandoned approach, not a shipped apostrophe fix or an open tracker for this failure.

## Supporting evidence

- crates/uv-virtualenv/src/virtualenv.rs:494–508: the template dispatch has a special activate.bat case only when relocatable is true. Ordinary activate.bat paths reach escape_posix_for_single_quotes(location_string). Lines 533–538 substitute that value into the activation template.
- crates/uv-shell/src/shlex.rs:22–38: the helper is explicitly for POSIX single-quoted strings and replaces every apostrophe with '"'"', matching the reporter's unexpected path fragment.
- crates/uv-virtualenv/src/activator/activate.bat:29 sets VIRTUAL_ENV from the substituted path in a batch for command. Line 66 prepends %VIRTUAL_ENV%\Scripts to PATH.
- crates/uv/tests/python/venv.rs:2129–2186: create_venv_apostrophe tests a directory named Testing's, but is restricted to Linux and executes a POSIX activation script. It does not cover cmd.exe or activate.bat.
- The diff for astral-sh/uv#8984 introduces the POSIX helper in the default template branch and adds that Linux-only test. Its merge date is November 15, 2024; changelogs/0.5.x.md:248–310 records it under uv 0.5.3. This substantially predates the reporter's uv 0.13.7.
- Maintainer comments on astral-sh/uv#8947 discuss POSIX single-quote rules and the concatenation strategy used by the helper.
- A maintainer comment on astral-sh/uv#9424 points to astral-sh/uv#9466 for batch handling. The PR's actual final state is closed, not merged; its final author comment says double quotes cannot occur in Windows filenames. That comment in the issue must not be treated as evidence of a released batch fix.

## Other candidates assessed

- astral-sh/uv#11828 and merged astral-sh/uv#11831 concern non-ASCII paths being decoded with the wrong Windows code page. The batch template already includes the code-page change; an ASCII apostrophe transformed into POSIX syntax is a different mechanism.
- astral-sh/uv#1763 concerns extended-length Windows path prefixes in PATH, not apostrophe rewriting.
- astral-sh/uv#23060 concerns fish executing commands from unescaped prompt/path input. The shell, malformed syntax, and observable failure differ from cmd.exe receiving an escaped path.
- astral-sh/uv#15294 tracks broader activation-test coverage, not this concrete Windows activation failure.

## Search scope and limitations

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Literal searches covered activate.bat, apostrophe, VIRTUAL_ENV, single quote, and escape_posix_for_single_quotes; conceptual and fix-oriented searches covered activation, escaping, quoting, batch, special characters, and area:environments. Inspected issue bodies, maintainer comments, timelines, PR reviews and diffs, the uv 0.5.3 changelog, and current activation code/tests. PR searches returned no hits, so historical PRs were recovered through issue references and inspected directly. Followed astral-sh/uv#8947 to astral-sh/uv#8984 and astral-sh/uv#9424 to astral-sh/uv#9466. Ruled out astral-sh/uv#11828 and its merged fix astral-sh/uv#11831 (non-ASCII code-page corruption), astral-sh/uv#1763 (Windows extended-length path prefixes), astral-sh/uv#23060 (fish prompt/path command execution), and astral-sh/uv#15294 (general activation-test coverage) as duplicate trackers. A final broader venv/quote issue query hit the GitHub API rate limit.

## Suggested next step

Give ordinary activate.bat paths batch-appropriate rendering instead of the POSIX escaping fallback. Validate on Windows with an explicit apostrophe-containing path: execute the generated batch activator, check VIRTUAL_ENV against the actual directory, and verify that Python resolves to the environment. Keep the existing POSIX apostrophe coverage. No further reproduction details are needed to establish the source-level bug.

Only this handoff README was updated. No checkout files were modified, no builds or tests were run, and no GitHub comments, labels, or other mutations were made.
