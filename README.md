# Multi step resolution

Issue: astral-sh/uv#20985

Classification: question

## Summary

The reporter asks how to resolve `package_a==1.0.2` with a one-time constraints file, then add
`package_b==1.3.0` while retaining compatible versions chosen in the first resolution. The desired
final lock contains `package_a==1.0.2`, `package_b==1.3.0`, `package_c==1.5.0`, and
`package_d==2.1.0`.

This can be done directly with the project interface. `uv add -c` applies external constraints to
that resolution without recording them as project dependencies. A later `uv add` uses compatible
versions from the existing lockfile as preferences, while replacing a locked version when the new
requirement is incompatible.

A constraint does not introduce a package by itself. Consequently, `package_b==1.2.0` in the first
constraints file has no effect because nothing in that first resolution requires `package_b`.

## Classification

This is a `question`, not a demonstrated uv defect. The report asks whether uv supports a workflow,
and a targeted reproduction confirms that the existing `uv add --constraint` behavior produces the
requested result. The issue is not the persistent-constraints enhancement in astral-sh/uv#16508:
persisting `package_b==1.2.0` into the second resolution would conflict with the requested
`package_b==1.3.0`.

## Reproduction

Outcome: **reproducible** on Linux x86_64 with uv 0.12.2, CPython 3.12.3, and an isolated temporary
project, package directory, cache, and virtual environment under `/tmp`.

The local flat package directory contained these sanitized wheels reconstructed from the report:

- `package-a==1.0.2`, depending on `package-c>=1.4.0,<2.0.0` and
  `package-d>=2.0.0,<3.0.0`.
- `package-b==1.3.0`, depending on `package-a>=1.0.0`, `package-c>=1.0.0,<2.0.0`, and
  `package-d==2.1.0`.
- `package-c==1.5.0` and `package-c==1.6.0`. The extra compatible 1.6.0 distinguishes retention of
  the first locked version from a fresh highest-version selection.
- `package-d==2.0.5` and `package-d==2.1.0`.

The constraints file was:

```text
package-b==1.2.0
package-c==1.5.0
package-d==2.0.5
```

The reproduction commands were:

```console
$ uv init --bare --python 3.12 project
$ cd project
$ uv add --no-index --find-links ../dist package-a==1.0.2 --constraint ../constraints.txt
$ uv add --no-index --find-links ../dist package-b==1.3.0
```

The first `uv add` resolved and installed `package-a==1.0.2`, `package-c==1.5.0`, and
`package-d==2.0.5`. It did not install constraints-only `package-b==1.2.0`, and the generated
`pyproject.toml` recorded only `package-a==1.0.2`.

The second `uv add` reported:

```text
+ package-b==1.3.0
- package-d==2.0.5
+ package-d==2.1.0
```

The final `uv.lock` contained exactly:

```text
package-a==1.0.2
package-b==1.3.0
package-c==1.5.0
package-d==2.1.0
```

Thus uv retained the compatible locked `package-c==1.5.0` even though 1.6.0 was available, and
changed `package-d` because the new direct dependency required 2.1.0.

Existing integration coverage confirms the two underlying behaviors, although no single test
models this exact two-step graph:

- `crates/uv/tests/project/edit.rs`, `add_requirements_file_constraints`, passes requirements and an
  external constraints file to `uv add`, then asserts that constrained versions are selected while
  the constraints are absent from both the project requirements and lockfile constraints.
- `crates/uv/tests/lock/lock.rs`, `lock_preference`, first locks `iniconfig==1.1.1`, loosens the
  project requirement, and asserts that 1.1.1 remains selected; it changes to 2.0.0 only when
  `--upgrade` is requested.

If the report's `package_d>=2.0.0<3.0.0` is literal metadata rather than abbreviated prose, it is
invalid and needs the comma used above: `package_d>=2.0.0,<3.0.0`.

## Draft response

Yes. You can replace the environment mutation, freeze, and import sequence with:

```console
$ uv init
$ uv add package_a==1.0.2 -c package_a_constraints-1-0-2.txt
$ uv add package_b==1.3.0
```

The first command applies the constraints for that resolution and writes the selections to
`uv.lock`, without persisting the constraints in `pyproject.toml`. The second command retains
compatible locked versions, so `package_c==1.5.0` remains selected, while `package_d` changes from
2.0.5 to 2.1.0 to satisfy `package_b`. The `package_b==1.2.0` constraint does not install
`package_b` during the first step because constraints only limit packages that are otherwise
required.

## Related

- astral-sh/uv#11986, “Add support for constraints in `uv add`” (closed), requested the exact
  capability to seed `uv.lock` from external constraints without writing them to
  `pyproject.toml`.
- astral-sh/uv#12209, “Add support for `-c` constraints in `uv add`” (merged), implemented
  astral-sh/uv#11986 and added the integration coverage cited above.
- astral-sh/uv#15020, “uv pip install don't change pyproject.toml and uv.lock. How can i sync this”
  (closed), is adjacent to the reporter's workaround: `uv pip install` does not update project
  metadata or `uv.lock`, while `uv add` does.
- astral-sh/uv#16508 tracks persistent external constraints, which are not wanted for this
  two-step resolution because the first-step `package_b==1.2.0` constraint must not remain active.
