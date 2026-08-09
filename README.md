# --find-links with a relative path in a requirements file fails with "relative URL without a base" (regression since 0.12.x)

Issue: astral-sh/uv#21016

Classification: bug

## Summary

The reporter says uv 0.12.0 through 0.12.3 rejects an existing relative `--find-links`
directory declared in `requirements.in` or `requirements.txt` with `relative URL without a
base`. They report the same parse failure from `uv pip compile` and `uv pip install -r`, while uv
0.11.26 worked and an absolute directory remains a workaround.

This behavior is already intended to work. Merged astral-sh/uv#20832 changed requirements-file
parsing to resolve an existing `--find-links` path against the directory containing the
requirements file, added an integration test using `uv pip install -r`, and was listed in the
0.12.1 release notes. Open astral-sh/uv#13239 is the broader design tracker for relative paths in
requirements files and is explicitly referenced by that pull request.

No open issue or pull request was found that already tracks the same reported regression. The
minimal compile example also did not reproduce with the published Linux 0.12.3 binary: both
`--find-links=./private_wheels/` and `--find-links ./private_wheels/` passed URL parsing and used the
directory, reaching only the expected resolution failure for the placeholder `somepkg`. The
reported macOS failure therefore needs a small amount of environment and literal-input evidence,
but a failure in the released macOS build would still be a regression of intended behavior.

## Draft response

Thanks for the report. astral-sh/uv#20832 is intended to cover this exact case and includes `uv pip
install -r` coverage for a relative `--find-links` directory. I also tried the minimal `uv pip
compile` example with the published 0.12.3 Linux binary; both `--find-links=./private_wheels/` and
the whitespace form were accepted, with resolution continuing past URL parsing.

Since your macOS result conflicts with that behavior, could you rerun the minimal example in a
fresh temporary directory and include the output of `uv --version --verbose`, `pwd`, `ls -ld
requirements.in private_wheels`, and the following?

```console
python -c 'from pathlib import Path; print(repr(Path("requirements.in").read_text())); print(Path("private_wheels").resolve(), Path("private_wheels").exists())'
```

That should show whether there is a platform-specific regression or an unrecorded path/input
detail.

## Classification

This is a `bug`, not an enhancement: resolving an existing requirements-file-relative
`--find-links` directory is established intended behavior in astral-sh/uv#20832. It is not a
duplicate of that merged pull request because the report claims that the fixed behavior has
returned in released versions, and no open issue or pull request currently tracks that regression.
The repository source and Linux release check do not confirm the failure, so the platform-specific
trigger and root cause remain unconfirmed.

## Related

- astral-sh/uv#20832 (merged pull request), “Resolve requirements-file find-links relative to the
  file”: the closest historical item. It changed the shared requirements parser to join a
  `--find-links` value to the containing requirements directory when that path exists. Its added
  integration test invokes `uv pip install -r requirements/requirements.txt` with
  `--find-links ./links`. This is the exact behavior that astral-sh/uv#21016 reports as regressed.
- astral-sh/uv#13239 (open issue), “Change relative path behaviors for `requirements` files”: the
  broader canonical discussion of whether relative paths should use the working directory or the
  containing file. astral-sh/uv#20832 explicitly references it while describing its own change as
  the narrow pip-compatible fix for an existing file-relative `--find-links` directory.

## Adjacent reports ruled out

- astral-sh/uv#7113 and astral-sh/uv#9681 concern the same `relative URL without a base` text when a
  configured local `--find-links` directory does not exist. Merged astral-sh/uv#9720 addressed the
  misleading missing-path handling. The new report explicitly says the directory exists, and the
  exact minimal setup used for validation also created it.
- astral-sh/uv#8057, fixed by merged astral-sh/uv#8061, involved a command-line path containing a
  space being split through `UV_FIND_LINKS`/argument value-separator behavior. It did not involve a
  relative entry inside a requirements file.
- astral-sh/uv#14367 was a requirements-file `--find-links` regression, but for a remote HTTP URL
  whose trailing slash and redirect behavior changed. Its fetch/404 symptoms and trigger differ
  from local path parsing here.
- astral-sh/uv#20786, fixed by merged astral-sh/uv#20802, added support for a local HTML file as a
  flat index. The new report uses an existing directory instead of an HTML file.

## Search scope and supporting evidence

Authenticated GitHub searches covered open and closed issues plus open, closed, and merged pull
requests. Literal searches used `relative URL without a base`, `--find-links`, `private_wheels`,
`pip compile`, `pip install`, `requirements`, and the reported 0.12 release range. Conceptual
searches covered local/relative directory resolution, containing-file semantics, working-directory
semantics, flat indexes, missing directories, and relative file URLs. Fix-oriented searches
followed astral-sh/uv#20832 to astral-sh/uv#13239 and inspected the issue/PR chains for the strongest
same-error and same-subsystem candidates listed above.

The current parser accepts both equals and whitespace option forms. Its `--find-links` branch joins
the expanded value to the containing requirements directory, converts an existing result to an
absolute file URL, and only falls back to URL parsing otherwise. The published 0.12.3 Linux artifact
was exercised in an isolated temporary directory with the report's compile layout; both option
forms reached dependency resolution rather than the reported URL parse error. This validation does
not rule out a macOS-specific failure, so the draft requests evidence that can isolate that
difference without asserting a cause.
