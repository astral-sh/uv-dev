# `uv python install` ignores `--default` when older version is specified

Issue: astral-sh/uv#21125

Classification: bug

## Summary

After `uv python install --default` installed CPython 3.14.7, the reporter ran
`uv python install --default 3.14.6`. uv installed 3.14.6 successfully, but the
`python` and `python3` executables in `~/.local/bin` continued to resolve to
3.14.7. The report is for uv 0.12.3 on macOS 15 x86_64.

This is a correctness problem in the interaction between `--default` and
transparent patch upgrades. The command help describes `--default` as using the
requested Python as the default and installing the unversioned executables.
The pull request that introduced the flag, astral-sh/uv#8650, further records
the intended behavior: `--default` should override executables managed by uv.

The current implementation does attempt to replace a different managed target
when `--default` is present, but it creates the bin executables through a
`PythonMinorVersionLink`. After link creation, `uv python install` selects the
highest installed patch for each minor-version key and makes that intermediary
link target the highest patch. With both 3.14.6 and 3.14.7 installed, the
unversioned executables therefore continue to resolve through the 3.14 link to
3.14.7. This minor-version indirection was introduced for transparent upgrades
by astral-sh/uv#13954.

No existing issue or pull request was found that tracks this stable
older-patch case. astral-sh/uv#15237 has the closest user-visible symptom, but
it concerns a prerelease minor-version request for which the unversioned links
were not created at all. That prerelease mechanism was fixed by
astral-sh/uv#16706 and is not the same problem.

## Draft response

Thanks for the clear reproduction. This is a bug: `--default` should make the
requested managed Python the target of `python` and `python3`, even when that
means selecting an older installed patch. The current implementation routes
those executables through the minor-version link, which is then set to the
highest installed patch, so requesting 3.14.6 leaves them resolving to 3.14.7.
We can track the stable older-patch case here; a regression test should cover
installing 3.14.7 first and then running
`uv python install --default 3.14.6`.

## Classification

Classify astral-sh/uv#21125 as a bug, not an enhancement or question. The
reported command completes successfully but does not perform the selection
expressed by `--default`. Repository evidence establishes both sides of the
conflict:

- astral-sh/uv#8650 says `--default` should override uv-managed `python` and
  `python3` executables.
- The current `create_bin_links` path treats a `--default` install as
  upgradeable and links through `PythonMinorVersionLink`.
- `highest_installations_by_minor_version_key` then selects 3.14.7 over 3.14.6
  and `ensure_minor_version_link` points the shared 3.14 intermediary at that
  highest installation.

This also explains the absence of a conflict warning: uv recognizes the
existing executables as managed and follows its normal replacement path, but
the replacement still uses the same minor-version intermediary. No open issue
or pull request already tracks this exact regression, so it is not a
duplicate.

## Related

- astral-sh/uv#8650 — merged pull request, “Add `uv python install
  --default`.” It introduced the flag. Its maintainer discussion explicitly
  establishes that `--default` should override uv-managed default executables.
  It also records the important distinction that an ordinary downgrade of the
  minor-version executable requires `--force`.
- astral-sh/uv#13954 — merged pull request, “Support transparent Python patch
  version upgrades.” It introduced the minor-version symlink or junction used
  by upgradeable bin installations. The pull request defines
  `PythonMinorVersionLink` as pointing to the highest installed patch for its
  minor-version key, directly matching the mechanism visible in current source.
- astral-sh/uv#15237 — open issue, “`uv python install --default 3.14` doesn't
  make 3.14 my global python.” It is the closest symptom-level report, but is
  adjacent rather than duplicate: its trigger was a prerelease minor request
  and its output omitted `python` and `python3` entirely. The corresponding
  prerelease handling was fixed later by astral-sh/uv#16706, whereas
  astral-sh/uv#21125 creates managed links that resolve to a newer stable patch.

## Search evidence

Literal searches covered `uv python install --default`, the exact versions
3.14.6 and 3.14.7, `.local/bin`, symlinks, and older-version wording.
Conceptual searches covered global or default Python selection, unversioned
executables, managed-link replacement, patch downgrades, latest-patch
preference, minor-version links, and transparent upgrades. Fix-oriented
searches covered closed issues and merged pull requests for executable
repointing and patch-upgrade behavior, including astral-sh/uv#8733,
astral-sh/uv#14247, and astral-sh/uv#14261 in addition to the closest items
above. astral-sh/uv#16696 was also inspected and ruled out as a duplicate
because it is the fixed prerelease-link creation case.
