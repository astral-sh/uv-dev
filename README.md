# uv pip install -r pylock.toml ignores `default-groups` entries not also listed in `dependency-groups`

Issue: astral-sh/uv#21917

Classification: bug

## Summary

The reported behavior is reproducible. When a PEP 751 lock lists a name in top-level
`default-groups` but not in `dependency-groups`, uv does not include that name in the
`dependency_groups` marker environment. This occurs whether `dependency-groups` is absent or lists
other names. A package guarded by the default group's marker is consequently omitted, while the
command exits successfully.

The behavior is specific to the default-only shape. With the same group also listed in
`dependency-groups`, uv selects the package. The current implementation is consistent with the
observation: the `pip install` and `pip sync` paths apply `lock.default_groups` as defaults, then
obtain concrete marker groups by filtering against `lock.dependency_groups`.

Explicit selection is not a workaround. The reporter corrected the original claim after retesting
the minimal lock on uv 0.12.17: `--group mygroup` also makes no changes when `mygroup` appears only
in `default-groups`. It selects the package only when the name is present in the lock's
`dependency-groups` array.

## Classification

This is a bug in the experimental pylock installation path, not a request for general dependency
group support. PEP 751 says `dependency_groups` should default to the set created from
`default-groups`, and also says names in `default-groups` should not need to be listed in
`dependency-groups`. The reproduced successful no-op therefore differs from the specified default
marker evaluation.

It is not a duplicate of astral-sh/uv#14740. That issue concerned rejecting the
`dependency_groups` marker during parsing; astral-sh/uv#21917 parses the marker and silently
evaluates it without the default-only group.

## Reproduction

Outcome: **reproducible** on Linux 6.17.0-1022-azure x86_64 with the installed uv 0.12.13
(x86_64-unknown-linux-gnu) and CPython 3.12.3.

The reproduction used a temporary directory and cache, a fresh virtual environment, and this
minimal `pylock.toml`:

```toml
lock-version = "1.0"
default-groups = ["mygroup"]
created-by = "reproduction"

[[packages]]
name = "mypy-extensions"
version = "1.1.0"
marker = "\"mygroup\" in dependency_groups"
requires-python = ">=3.8"

[[packages.wheels]]
name = "mypy_extensions-1.1.0-py3-none-any.whl"
url = "https://files.pythonhosted.org/packages/79/7b/2c79738432f5c924bef5071f933bcc9efd0473bac3b4aa584a6f7c1c8df8/mypy_extensions-1.1.0-py3-none-any.whl"

[packages.wheels.hashes]
sha256 = "1be4cccdb0f2482337c4743e60421de3a356cd97508abadd57d47403e94f5505"
```

Commands:

```console
$ UV_CACHE_DIR=/tmp/uv-issue-21917/cache uv venv .venv --python /usr/bin/python3
$ UV_CACHE_DIR=/tmp/uv-issue-21917/cache uv pip install --preview-features pylock --dry-run -r pylock.toml
Checked in 0.00ms
Would make no changes
$ UV_CACHE_DIR=/tmp/uv-issue-21917/cache uv pip install --preview-features pylock -r pylock.toml
Checked in 0.00ms
$ UV_CACHE_DIR=/tmp/uv-issue-21917/cache uv pip show mypy-extensions
warning: Package(s) not found for: mypy-extensions
```

The install command exited 0 and did not install `mypy-extensions`. As a targeted control, adding
`dependency-groups = ["mygroup"]` while retaining the same `default-groups` and package marker made
the dry run report:

```console
Would download 1 package
Would install 1 package
 + mypy-extensions==1.1.0
```

On the installed uv 0.12.13, explicitly passing `--group mygroup` to the original default-only file
also produced `Would make no changes`. The reporter subsequently corrected the original workaround
claim and independently confirmed the same result on uv 0.12.17.

The reporter later expanded and corrected the no-flag controls, varying only the two top-level group
fields. The results below are reporter-supplied for pip 26.2.1 and uv 0.9.17, 0.12.17, and 0.12.18;
the first and third uv cases are also consistent with the independent uv 0.12.13 reproduction and
control above.

| `dependency-groups` | `default-groups` | pip 26.2.1 | uv 0.9.17 / 0.12.17 / 0.12.18 |
| --- | --- | --- | --- |
| Absent | `["mygroup"]` | Installs | No changes |
| `["othergroup"]` | `["mygroup"]` | Installs | No changes |
| `["mygroup"]` | `["mygroup"]` | Installs | Installs |
| `["mygroup"]` | Absent | No changes | No changes |

The second row isolates the defect to membership of the default name in `dependency-groups`, rather
than merely the presence or absence of that key. The corrected third row also confirms that uv does
honor `default-groups` when the same name is declared in `dependency-groups`.

The reporter also observed on uv 0.12.17 that `--group totallybogus` is accepted without a warning
and exits successfully without changes. This diagnostic behavior was not independently reproduced
as part of the existing reproduction, but it is consistent with the implementation filtering
requested names against the lock's declared `dependency_groups`.

Existing integration coverage does not exercise the reported shape. The test
`crates/uv/tests/pip_install/pip_install.rs::pep_751_groups` verifies default group marker
evaluation, extras, and explicit groups, but declares `default` in both `dependency-groups` and
`default-groups`. It therefore covers the successful control case, not a name present only in
`default-groups`.

## Related

- astral-sh/uv#14740 (closed) — Reported that `uv pip install -r pylock.toml` rejected a PEP 751
  `dependency_groups` marker during parsing.
- astral-sh/uv#14755 (merged) — Added parsing and evaluation for `extras` and `dependency_groups`
  markers in `uv pip install` and `uv pip sync`; its integration fixture lists the default group in
  both top-level group fields.
- pypa/pipx#2052 (merged) — Fixed pipx manifest lock/sync by emitting the selected name in both
  `dependency-groups` and `default-groups`. Its compatibility checks found that this dual-listed
  shape installs under pip 26.2.1 and uv 0.9.17/0.12.17. It is a practical downstream workaround,
  not evidence that uv handles a default name absent from `dependency-groups`.

## Implementation evidence

- `crates/uv-lock/src/lock/export/pylock_toml.rs` deserializes `default_groups` separately from
  `dependency_groups`.
- `crates/uv/src/commands/pip/install.rs` and `crates/uv/src/commands/pip/sync.rs` apply
  `lock.default_groups`, then call `group_names(lock.dependency_groups.iter())` before resolving the
  pylock file. This is consistent with a default-only name being filtered out.
