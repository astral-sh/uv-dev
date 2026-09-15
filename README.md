# 0.12.14: `uv pip install --target=.` fails for every wheel with "The wheel is invalid: Wheel directory entry escapes its destination"

Issue: astral-sh/uv#21694

Classification: bug

## Summary

uv 0.12.14 fails to install ordinary wheels when `uv pip install` receives `--target=.` or
`--target=./`. The reported annotated-types reproduction succeeds with 0.12.13, and 0.12.14 also
succeeds when the same directory is expressed as an absolute path or when a non-dot relative target
is used. The failure occurs after preparation with `The wheel is invalid: Wheel directory entry
escapes its destination` on a normal wheel directory such as the `.dist-info` directory.

No existing issue or pull request tracks this exact literal-dot target regression. The closest
evidence is the merged change that introduced destination validation, astral-sh/uv#21569, and the
separate 0.12.14 symlink-destination regression in astral-sh/uv#21692.

## Draft response

Thanks for the reproducer. This is an unintended regression in 0.12.14. `--target=.` remains a
literal relative root, and the destination validation added by astral-sh/uv#21569 consequently
rejects normal wheel directories beneath it. The equivalent absolute target works, so
`--target="$PWD"` is a workaround. We should normalize the target path before wheel validation and
add regression coverage for the dot forms.

## Classification

This is a bug rather than an enhancement or question. The command worked in 0.12.13, the target
directory itself is valid, and equivalent spellings of the same destination still work in 0.12.14.
Repository source confirms the correctness problem: `uv_python::Target` stores the supplied
`PathBuf` unchanged and uses it as the wheel scheme root. The validation added by
astral-sh/uv#21569 calls `normalize_path_under` for each wheel directory. That helper normalizes a
root of `.` to an empty path and explicitly rejects an empty root, so a normal child such as a
wheel's `.dist-info` directory is reported as escaping. This mechanism matches both the exact error
and the literal-dot-only trigger.

This is not a duplicate. No open issue or pull request was found for the same `--target=.` failure,
and there is no prior closed fix whose regression is being tracked. astral-sh/uv#21692 shares the
introducing validation but exercises a different branch: it rejects a real pre-existing symlink in
an installation scheme rather than mishandling the lexical dot root.

## Related

- astral-sh/uv#21569 — “Reject symlinked wheel installation destinations” (merged pull request).
  This added `ValidatedWheelDestination` and the exact `Wheel directory entry escapes its
  destination` error path. It merged before the 0.12.14 release and directly explains why the
  behavior changed from 0.12.13. It is the introducing change, not a fix created in response to this
  issue.
- astral-sh/uv#21692 — “0.12.14: `uv pip install --system` fails in official `python:*` Docker
  images — \"Cannot install into symlinked directory: /usr/local/man\"” (open issue). This is a
  sibling regression caused by astral-sh/uv#21569. It has the same release boundary and wheel
  destination-validation subsystem, but its trigger is a genuine `/usr/local/man` symlink and its
  error is `Cannot install into symlinked directory`, so it should remain a separate report.

## Supporting evidence

- Literal searches covered the complete error, `escapes its destination`, `--target=.`,
  `--target=./`, `The wheel is invalid`, `directory entry`, and the reported 0.12.14 release.
- Conceptual searches covered relative targets, current-working-directory targets, wheel
  destination traversal, symlinked installation roots, invalid wheel directories, and normalized
  path handling across open and closed issues plus open, closed, and merged pull requests.
- Fix-oriented searches covered recent merged pull requests and 0.12.14 destination-validation
  changes. No existing fix or pull request for astral-sh/uv#21694 was found.
- astral-sh/uv#21644 was inspected because it also rejects a directory entry, but it concerns ZIP
  extraction of a DEFLATE-compressed empty directory with a data descriptor in 0.12.13, not wheel
  installation destination validation.
- astral-sh/uv#9656 was inspected because it reports an invalid-wheel error for a directory, but it
  concerns a package placing a directory under `.data/scripts`; maintainers concluded that wheel
  layout was invalid. It does not depend on `--target=.` or the 0.12.14 validation change.
- astral-sh/uv#21255 and astral-sh/uv#5631 were also inspected. The former concerns console-script
  placement through a virtual-environment `lib` symlink, while the latter concerns editable
  workspace members under `--target`; neither matches this error or trigger.
