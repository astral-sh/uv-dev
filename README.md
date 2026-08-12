# Unformatted error messages on `uv version --bump`

Issue: astral-sh/uv#21063

Classification: bug

## Summary

On uv 0.12.3 for Linux, `uv version --bump foo` rejects the unknown bump component with
``error: invalid bump component `foo```, but the diagnostic is not styled like normal CLI errors and
does not end in a newline. The next shell prompt is consequently printed on the same line.

The behavior is reproducible with the installed uv 0.12.3 in an empty temporary directory. The
command exits with status 2 and writes exactly 35 bytes to stderr, with neither ANSI styling nor a
final newline. A pseudo-terminal comparison against clap's missing-value error confirms that normal
argument diagnostics are styled and newline-terminated in the same environment.

The current `VersionBumpSpecValueParser` in `crates/uv-cli/src/lib.rs` contains the exact diagnostic
and converts parser failures into `clap::Error::raw(ErrorKind::InvalidValue, message)`. The custom
parser was introduced by astral-sh/uv#16555 while adding explicit values such as
`--bump patch=10`. Existing integration tests cover invalid numeric, empty, and disallowed stable
values, but do not cover an unknown component such as `foo`.

No existing issue or pull request was found that tracks the same presentation defect. The closest
history is the custom parser's feature issue and implementation.

## Reproduction

Outcome: **reproducible**.

The reproduction used the installed `uv 0.12.3 (x86_64-unknown-linux-gnu)` on Linux
6.17.0-1020-azure x86_64. It was run in a newly created directory under `/tmp`, with uv's cache and
Python installation directories also redirected there. No project files or configuration were
needed:

```console
$ uv version --bump foo
error: invalid bump component `foo`$ echo $?
2
```

Capturing stderr showed that stdout was empty and stderr was exactly these 35 bytes:

```text
65 72 72 6f 72 3a 20 69 6e 76 61 6c 69 64 20 62
75 6d 70 20 63 6f 6d 70 6f 6e 65 6e 74 20 60 66
6f 6f 60
```

There is no trailing `0a` byte. In a pseudo-terminal with `NO_COLOR` unset and
`TERM=xterm-256color`, the reported command still emitted zero ANSI escape sequences and no final
newline. As a control, `uv version --bump` emitted clap's missing-value diagnostic with 26 ANSI
escape sequences and ended in a newline. This reproduces both reported symptoms independently of
shell prompt rendering.

The report used Python 3.14.6, while the reproduction host has Python 3.12.3. The failure occurs
during CLI value parsing before project discovery or Python selection, and the empty-directory
reproduction reaches the reported output without invoking Python.

Nearby integration coverage is in `crates/uv/tests/it/version.rs`:

- `bump_stable_with_value_fails` covers `--bump stable=1`.
- `bump_empty_value_fails` covers `--bump patch=`.
- `bump_invalid_numeric_value_fails` covers `--bump dev=foo`.

Those tests exercise other failures from the same custom parser, but none passes an unknown
component such as `--bump foo` or asserts the terminal styling and final stderr byte for that case.

## Draft response

Thanks for the clear reproduction. This is a bug. The invalid component goes through the custom
`--bump` value parser added in astral-sh/uv#16555, and that path currently constructs a raw clap
error; we also do not have an integration test for an unknown bump component.

A focused PR would be welcome. Please add regression coverage for `uv version --bump foo` that
checks the formatted diagnostic, including the trailing newline and styling when color is forced,
and update the parser so the error is rendered consistently with other CLI argument errors.

## Classification

This is a bug because uv emits malformed user-facing output for an invalid CLI value: missing
styling is inconsistent with its other argument errors, and the missing trailing newline visibly
corrupts the following prompt. Both symptoms were observed directly with the reported uv version;
the source inspection identifies the relevant implementation path but is not being used as a
substitute for behavioral reproduction.

This is not a duplicate. Searches found no open or closed issue and no open, closed, or merged pull
request that tracks the same missing formatting and newline. astral-sh/uv#16427 and
astral-sh/uv#16555 concern the feature that introduced the custom parser, not this defect. There is
also no evidence that a previously fixed formatting bug has returned.

## Search and evidence

The report was decomposed into the command and subsystem (`uv version --bump` and bump-value
parsing), trigger (an unknown component), exact diagnostic (`invalid bump component`), and two
observable symptoms (no styling and no trailing newline). Authenticated GitHub searches covered
open and closed issues and open, closed, and merged pull requests.

Literal searches included the exact diagnostic, `version --bump`, invalid bump/component terms,
unformatted errors, color, newline, trailing newline, and prompt concatenation. Conceptual searches
included CLI and argument error formatting, invalid-value and possible-value diagnostics, clap,
custom/value parsers, raw errors, styled/colored output, and issues carrying the repository's
`error messages` label. Fix-oriented searches included the issue number, bump-component terms,
formatting/color/newline combinations, and historical merged pull requests.

The broader original feature, astral-sh/uv#6298, was inspected and ruled out because it tracks
adding project version reading and bumping, not malformed diagnostics. astral-sh/uv#16427 was
inspected through its discussion and closing pull request; it requests explicit bump values and
does not report the presentation failure. The comments, reviews, files, and patch for
astral-sh/uv#16555 confirm that it introduced the custom `VersionBumpSpecValueParser`, the raw clap
error construction, and the exact diagnostic now reported.

## Related

- astral-sh/uv#16555 (merged pull request), “Allow explicit values with `uv version --bump`” —
  direct implementation provenance. It replaced the built-in bump enum parser with
  `VersionBumpSpecValueParser` and introduced the exact diagnostic through `clap::Error::raw`. Its
  purpose was explicit numeric bump values, not error presentation, so it does not already track
  astral-sh/uv#21063.
- astral-sh/uv#16427 (closed issue), “Allow setting a custom value for `uv version --bump`” — the
  enhancement implemented by astral-sh/uv#16555. It is adjacent rather than duplicate: it requests
  custom component values and discusses their CLI syntax, with no report of missing styling or a
  trailing newline.
