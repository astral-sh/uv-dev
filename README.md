# Double `requires-python` exclusion gets ignored

Issue: astral-sh/uv#21036

Classification: bug

## Summary

The reported behavior is reproducible with uv 0.12.3. For a project declaring:

```toml
requires-python = ">=3.10, !=3.11.*, !=3.12.*, <3.14"
```

`uv lock` writes `requires-python = ">=3.10, <3.14"` to `uv.lock`, dropping both
wildcard exclusions and widening the lockfile's supported Python set to include Python 3.11 and
3.12. A control containing only the Python 3.11 wildcard exclusion retains it. The reporter's
`[tool.uv].environments` workaround is therefore avoiding an observed lockfile conversion bug.

## Classification

`bug` is appropriate. The declared constraint is valid, and the generated lockfile does not
preserve its meaning. The behavior was observed directly with the reported uv version rather than
inferred from source.

The implementation is consistent with the symptom: workspace `requires-python` intersection
converts the specifiers to ranges and reconstructs specifiers through
`VersionSpecifiers::from_release_only_bounds`. That routine handles an exact-version gap or a
single-minor wildcard gap but warns in source that it ignores unsupported wider gaps. Two
consecutive excluded minors form such a wider gap. This is supporting context, not a separately
confirmed root-cause analysis.

## Reproduction

Outcome: **reproducible**.

Environment:

- uv 0.12.3 (`x86_64-unknown-linux-gnu`), matching the reported uv version
- Linux x86_64 (Azure kernel 6.17.0-1020-azure; the report used WSL2 Linux x86_64)
- CPython 3.12.3 at `/usr/bin/python3`

In a new temporary directory, create this dependency-free fixture:

```toml
[project]
name = "double-exclusion"
version = "0.1.0"
requires-python = ">=3.10, !=3.11.*, !=3.12.*, <3.14"
```

Then run with temporary, isolated cache state:

```console
$ UV_CACHE_DIR="$PWD/cache" uv lock --offline --no-config
Using CPython 3.12.3 interpreter at: /usr/bin/python3
Resolved 1 package in 1ms

$ sed -n '1,8p' uv.lock
version = 1
revision = 3
requires-python = ">=3.10, <3.14"

[[package]]
name = "double-exclusion"
version = "0.1.0"
```

`uv lock --locked --offline --no-config` subsequently succeeds with that widened lockfile.

As a control, changing the declaration to
`requires-python = ">=3.10, !=3.11.*, <3.14"` and locking under the same conditions produces:

```toml
requires-python = ">=3.10, !=3.11.*, <3.14"
```

Existing integration coverage is in `crates/uv/tests/lock/lock.rs`, test
`lock_requires_python_not_equal`. Its fixture combines two individual patch exclusions
(`!=3.10.9`, `!=3.10.10`) with one wildcard-minor exclusion (`!=3.11.*`) and snapshots all three
unchanged in `uv.lock`; it does not cover two consecutive wildcard-minor exclusions. No existing
test found covers the reported trigger.

## Draft response

Thanks, this is reproducible with uv 0.12.3. A minimal dependency-free project with the reported
constraint writes `requires-python = ">=3.10, <3.14"` to `uv.lock`, while the equivalent control
with one wildcard exclusion preserves it. The existing not-equal lock test covers exact exclusions
and one wildcard-minor exclusion, but not consecutive wildcard-minor exclusions. Your
`[tool.uv].environments` setting remains a valid workaround while this is fixed.

## Related

### astral-sh/uv#7862 — `requires-python` specification not correctly resolved (closed)

This is the closest historical issue. Its project constraint contained an exact-version exclusion
that was incorrectly reconstructed during `uv lock`. It concerns the same correctness area, but
not the consecutive wildcard-minor trigger reproduced here.

### astral-sh/uv#7897 — Fix handling of `!=` intersections in `requires-python` (merged)

This pull request closed astral-sh/uv#7862 and established that exclusions must survive
`requires-python` intersection.

### astral-sh/uv#8060 — Add gap-preserving range-to-PEP 440 routine (merged)

This follow-up introduced the current range-to-specifier conversion. The existing regression test
covers multiple exact exclusions and a single wildcard-minor exclusion, but not a wider gap formed
by consecutive wildcard-minor exclusions.

### astral-sh/uv#20816 — `project.requires-python` not handled correctly (closed)

This is adjacent context, not a duplicate. It documents that commas are logical AND and recommends
the same form for skipping one minor version. The reporter is already using the correct form; the
new trigger is skipping two consecutive minor versions.
