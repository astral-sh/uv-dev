# --find-links with a relative path in a requirements file fails with "relative URL without a base" (regression since 0.12.x)

Issue: astral-sh/uv#21016

Classification: bug

## Summary

The reporter says uv 0.12.0 through 0.12.3 rejects an existing relative `--find-links`
directory declared in `requirements.in` or `requirements.txt` with `relative URL without a
base`. They report the same parse failure from `uv pip compile` and `uv pip install -r` on macOS
x86_64 with Python 3.10.17, while uv 0.11.26 worked and an absolute directory remains a workaround.

The reported minimal example was not reproducible with the published Linux x86_64 binaries. Its
relative path passed parsing on uv 0.11.26 and every release from 0.12.0 through 0.12.3. With a
trusted wheel in the relative directory, uv 0.12.3 successfully compiled and dry-run installed the
requirement. The macOS-specific observation therefore needs literal input and environment evidence;
no root cause has been confirmed.

## Reproduction

Outcome: `not_reproducible`.

Environment: Linux x86_64, Python 3.12.3, installed `uv 0.12.3
(x86_64-unknown-linux-gnu)`. All files, caches, virtual environments, and downloaded comparison
binaries were isolated under `/tmp`.

The report's layout was reconstructed safely, adding `--no-index` to avoid resolving the untrusted
placeholder package from the network:

```console
$ mkdir private_wheels
$ printf '%s\n' '--find-links=./private_wheels/' somepkg > requirements.in
$ uv pip compile requirements.in -o requirements.txt --no-index
  × No solution found when resolving dependencies:
  ╰─▶ Because somepkg was not found in the provided package locations
      and you require somepkg, we can conclude that your requirements are
      unsatisfiable.
```

`uv pip install --dry-run -r requirements.in --no-index` in a temporary virtual environment reached
the same resolution error. Neither command produced the reported URL parse error.

The same compile check was run with published Linux binaries for 0.11.26, 0.12.0, 0.12.1, 0.12.2,
and 0.12.3. Every version accepted the relative `--find-links` value and reached only the expected
missing-package resolution failure.

To prove that the directory was used rather than merely accepted, the trusted repository fixture
`test/links/ok-1.0.0-py3-none-any.whl` was copied into `private_wheels`, and the requirement was
changed to `ok==1.0.0`. On installed uv 0.12.3, both commands succeeded:

```console
$ uv pip compile requirements.in -o requirements.txt
Resolved 1 package in [TIME]
ok==1.0.0
    # via -r requirements.in
$ uv pip install --dry-run -r requirements.in --python .venv/bin/python
Resolved 1 package in [TIME]
Would download 1 package
Would install 1 package
 + ok==1.0.0
```

Additional uv 0.12.3 compile variants all succeeded: `--find-links=./private_wheels/`,
`--find-links=private_wheels/`, and `--find-links ./private_wheels/`, each with both relative and
absolute arguments for the requirements file.

The 0.12.1 release notes and merged astral-sh/uv#20832 address a narrower pre-existing behavior:
when the requirements file is in a subdirectory and only that subdirectory contains the relative
links directory, uv 0.11.26 and 0.12.0 resolve from the caller's working directory and emit the
reported URL error; uv 0.12.1, 0.12.2, and 0.12.3 resolve from the containing directory and succeed.
That version boundary was reproduced separately and is consistent with the change, but it is not
the claimed regression and does not match the report's same-directory minimal example.

Existing coverage is in
`crates/uv/tests/pip_install/pip_install.rs`, test
`find_links_relative_to_requirements_file`. It creates
`requirements/requirements.txt`, places a wheel under `requirements/links`, declares
`--find-links ./links`, runs `uv pip install -r requirements/requirements.txt` from the parent, and
asserts successful installation. The test setup and snapshot directly cover containing-file
resolution for `pip install`. Compile behavior was observed directly above.

To investigate the reported macOS result, maintainers still need output from a fresh temporary
directory showing `uv --version --verbose`, `pwd`, `ls -ld requirements.in private_wheels`, and a
literal representation of the requirements file plus the resolved path and existence result for
`private_wheels`. Those details would distinguish a platform-specific binary issue from an
unreported path or input difference.

## Draft response

Thanks for the report. I could not reproduce this with the published Linux x86_64 binaries. The
same-directory minimal fixture passed `--find-links` parsing on uv 0.11.26 and every version from
0.12.0 through 0.12.3. With a local test wheel, uv 0.12.3 successfully completed both `uv pip
compile` and `uv pip install --dry-run -r`; the equals, whitespace, `./`, bare relative path, and
absolute requirements-file variants also succeeded.

astral-sh/uv#20832 covers the stricter case where the requirements file and links directory are in
a subdirectory of the caller's working directory. I confirmed that case fails before 0.12.1 and
succeeds from 0.12.1 onward.

Could you rerun the minimal example in a fresh temporary directory and include `uv --version
--verbose`, `pwd`, `ls -ld requirements.in private_wheels`, and this output?

```console
python -c 'from pathlib import Path; print(repr(Path("requirements.in").read_text())); print(Path("private_wheels").resolve(), Path("private_wheels").exists())'
```

That should reveal the unrecorded difference or provide evidence of a macOS-specific issue.

## Classification

This remains classified as a `bug` report because the claimed behavior conflicts with established
intended behavior, but the reported regression is not reproducible from the supplied example. The
published Linux builds, current implementation, and existing integration test agree that an
existing requirements-file-relative `--find-links` directory is supported. A macOS-specific trigger
or literal input difference remains possible but unconfirmed.

## Related

- astral-sh/uv#20832 (merged pull request), “Resolve requirements-file find-links relative to the
  file”: merged on 2026-07-30 and released in 0.12.1. It changed the shared requirements parser to
  join a `--find-links` value to the containing requirements directory before checking existence,
  and added the integration test described above.
- astral-sh/uv#13239 (open issue), “Change relative path behaviors for `requirements` files”: the
  broader canonical discussion of working-directory versus containing-file semantics.
- astral-sh/uv#7113 and astral-sh/uv#9681 concern the same error when a configured local
  `--find-links` directory does not exist; merged astral-sh/uv#9720 addressed misleading
  missing-path handling. The reconstructed fixture explicitly created the directory.
- astral-sh/uv#8057, fixed by merged astral-sh/uv#8061, involved a command-line path containing a
  space being split through environment-variable argument handling, not a requirements-file path.
- astral-sh/uv#14367 was a requirements-file `--find-links` regression involving an HTTP URL,
  redirects, and trailing slashes rather than a local directory.
- astral-sh/uv#20786, fixed by merged astral-sh/uv#20802, added support for a local HTML file as a
  flat index. The report uses an existing directory.
