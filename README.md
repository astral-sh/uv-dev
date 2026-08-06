# Multi step resolution

Issue: astral-sh/uv#20985

Classification: question

## Summary

The reporter wants to resolve `package_a==1.0.2` under a one-time constraints file, then add
`package_b==1.3.0` and retain the compatible versions selected in the first step. Their desired
lock keeps `package_c==1.5.0`, replaces the constraints-only `package_b==1.2.0` with the explicitly
requested `package_b==1.3.0`, and changes `package_d==2.0.5` to the newly required
`package_d==2.1.0`. They currently achieve this by mutating an environment with `uv pip install`,
freezing it, and importing the frozen versions with `uv add -r`.

The project interface already supports this workflow. `uv add -c` applies an external constraints
file to that invocation's resolution without adding the constraints to `pyproject.toml`. The
selected versions are stored in `uv.lock`. On a later `uv add`, uv prefers compatible versions from
the existing lockfile, changing them only when the new requirement is incompatible or an upgrade is
explicitly requested. Therefore the direct sequence is:

```console
$ uv init
$ uv add package_a==1.0.2 -c package_a_constraints-1-0-2.txt
$ uv add package_b==1.3.0
```

With the dependency metadata described in the report, this should retain `package_c==1.5.0` and
move `package_d` to `2.1.0`. A constraints entry does not install a package by itself, so the first
step's `package_b==1.2.0` entry only has an effect if something in that first resolution requires
`package_b`. The second command explicitly adds `package_b==1.3.0` after the external constraints
are no longer active.

If the reported `package_d>=2.0.0<3.0.0` text is literal rather than abbreviated prose, it needs a
comma: `package_d>=2.0.0,<3.0.0`.

## Draft response

Yes. You can do this directly with the project interface, without installing into an environment,
freezing it, and importing the result:

```console
$ uv init
$ uv add package_a==1.0.2 -c package_a_constraints-1-0-2.txt
$ uv add package_b==1.3.0
```

`uv add -c` uses the constraints file for that resolution, but does not persist those constraints
to `pyproject.toml`. The selected versions are written to `uv.lock`. On the second `uv add`, uv
prefers compatible versions already in the lockfile, so `package_c==1.5.0` should remain selected;
`package_d==2.0.5` is incompatible with `package_b`'s `package_d==2.1.0` requirement, so it should
move to `2.1.0`. This constraints workflow was added in astral-sh/uv#11986 and implemented by
astral-sh/uv#12209.

Also, if `package_d>=2.0.0<3.0.0` is copied literally from the package metadata, change it to
`package_d>=2.0.0,<3.0.0`.

## Classification

This is a `question`. The report asks whether uv can perform a particular multi-step resolution and
does not show incorrect behavior. The requested result follows existing, documented behavior:
external constraints are accepted by `uv add`, and versions in an existing output lockfile are
preferences that remain selected unless a later requirement makes them incompatible. No new
capability is required for the described sequence.

It is not a duplicate because the closest canonical issue, astral-sh/uv#11986, is a closed feature
request whose implementation supplies the answer rather than an existing discussion that needs to
centralize this support question. It is not the persistent-constraints enhancement tracked by
astral-sh/uv#16508: persistence would keep `package_b==1.2.0` active and conflict with the desired
second-step `package_b==1.3.0`.

## Related

- astral-sh/uv#11986, “Add support for constraints in `uv add`” (closed): This is the canonical
  issue for the exact capability. It requests `uv add -r requirements.in -c requirements.txt` so a
  constraints file can seed `uv.lock` without being written to `pyproject.toml`.
- astral-sh/uv#12209, “Add support for `-c` constraints in `uv add`” (merged): This implemented
  astral-sh/uv#11986. Its integration test verifies that external constraints control the resolved
  versions, are not recorded as project requirements or lockfile constraints, and leave a valid
  lockfile for subsequent project commands.
- astral-sh/uv#15020, “uv pip install don't change pyproject.toml and uv.lock. How can i sync this”
  (closed): This is adjacent to the reporter's current workaround. A maintainer explains that
  `uv pip install` does not update project metadata or `uv.lock`, recommends `uv add`, and describes
  `uv pip freeze | uv add --requirements -` only as a simulation when direct `uv add` cannot be
  used.

## Supporting evidence

- The current `uv add` CLI documentation states that `-c`/`--constraints` files control versions
  during dependency resolution, are not added to `pyproject.toml`, and are equivalent to pip's
  constraints option.
- The migration guide explicitly recommends `uv add -r requirements.in -c requirements.txt` to
  preserve previously locked versions when producing `uv.lock`.
- The resolution documentation states that uv prefers versions in an existing `uv.lock`, and that
  they do not change unless an incompatible version or an explicit upgrade is requested.
- Source in the lock operation represents a still-usable existing lockfile as version preferences;
  source in the candidate selector selects a matching lockfile preference before looking for a new
  candidate.

## Search coverage and exclusions

Searches covered the literal command and file vocabulary (`uv add`, constraints, `uv lock`,
`pip freeze`, and requirements import), the conceptual behaviors (preserving existing or locked
versions, dependency preferences, minimal changes, and transitive upgrades), and historical merged
fixes for project constraint support. Open and closed issues and open, closed, and merged pull
requests were included.

astral-sh/uv#16508 and astral-sh/uv#12490 were inspected but excluded because they request
persistent or sync-time external constraints, unlike the one-time first-step constraint needed
here. astral-sh/uv#14011 was also excluded: despite its similar title about a pinned version during
`uv add`, its actual issue concerns an isolated build dependency, not reuse of ordinary lockfile
versions. astral-sh/uv#8585 concerns conditional automation for bumping minimum versions across
repositories rather than this supported incremental add workflow.
