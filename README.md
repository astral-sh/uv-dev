# uv lock fails to pick up direct dependencies

Issue: astral-sh/uv#21824

Classification: bug

## Summary

The reported behavior is reproducible. A dynamic version forces uv to prepare backend metadata. With a newline-terminated `VERSION` file, the reported Hatch version pattern `(?P<version>[^']+)` captures the trailing newline. Hatchling 1.32.3 writes that raw value into the `Version` core-metadata header, placing a blank line before `Requires-Python` and `Requires-Dist`. uv then resolves the separately read dependency group but omits the static project dependencies.

Hatchling 1.32.3 was published on 2026-09-17, matching the onset in the report. Its change from pypa/hatch#2398 preserves the original version string in core metadata. Hatchling 1.32.0 normalized the extracted value before writing metadata and does not exhibit the failure.

## Reproduction

Outcome: **reproducible**.

The report used uv 0.12.16, Python 3.14.7, and Linux under WSL2. The isolated reproduction used the published uv 0.12.16 Linux x86-64 binary, CPython 3.12.3, a dedicated cache and tool directory under `/tmp`, and no user configuration. Python 3.12 was used with `requires-python = ">=3.12"`; the failure is in generated core metadata and reproduced independently of the reported Python 3.14 environment.

Minimal `pyproject.toml`:

```toml
[project]
name = "repro"
dynamic = ["version"]
requires-python = ">=3.12"
dependencies = ["anyio==4.3.0", "idna==3.6"]

[dependency-groups]
dev = ["iniconfig==2.0.0"]

[build-system]
requires = ["hatchling==1.32.3"]
build-backend = "hatchling.build"

[tool.hatch.version]
path = "VERSION"
pattern = "(?P<version>[^']+)"

[tool.hatch.build.targets.wheel]
packages = ["src/repro"]
```

`VERSION` contained `0.1.0` followed by a newline. Running `uv lock --upgrade --python /usr/bin/python3.12 --no-python-downloads` with uv 0.12.16 succeeded with `Resolved 2 packages`. The lock contained only `repro` and the dev dependency `iniconfig`; `anyio`, `idna`, and `anyio`'s transitive dependency `sniffio` were absent.

The wheel built with Hatchling 1.32.3 contained:

```text
Metadata-Version: 2.5
Name: repro
Version: 0.1.0

Requires-Python: >=3.12
Requires-Dist: anyio==4.3.0
Requires-Dist: idna==3.6
```

The blank line terminates the metadata headers, so uv's `mailparse`-based parser does not expose the later lines as `Requires-Dist` fields. Pinning `hatchling==1.32.0` removed that blank line and uv 0.12.16 resolved 5 packages, including both direct dependencies and `sniffio`. Keeping Hatchling 1.32.3 but changing the pattern to `(?P<version>\\S+)` also resolved all 5 packages. uv 0.12.13 reproduced the omission with Hatchling 1.32.3, so the evidence does not indicate a uv 0.12.16 code regression.

Relevant existing tests do not cover this exact integration. `crates/uv/tests/lock/lock.rs::lock_dynamic_version` covers locking a project with a dynamic version but declares an empty static dependency list. `crates/uv-pypi-types/src/metadata/metadata_resolver.rs::test_parse_metadata` covers parsing normal headers and a description body after a blank line, but not backend metadata in which dependency-looking lines are placed after an unintended early blank line.

## Draft response

Thanks for the detailed log. I can reproduce this, and the immediate regression comes from Hatchling 1.32.3 rather than a change to your dependency declarations.

Your version regex also captures the trailing newline in `VERSION`. pypa/hatch#2398 changed Hatchling to preserve the raw version string in core metadata, so Hatchling 1.32.3 writes that newline into the `Version` header. The resulting blank line ends the metadata header section before `Requires-Dist`, which is why uv sees no project dependencies while the separately read dependency group still works.

As an immediate workaround, pin the known-good backend version and regenerate the lockfile:

```toml
[build-system]
requires = ["hatchling==1.32.0"]
build-backend = "hatchling.build"
```

Then run `uv lock -U` again. You can alternatively change the version extraction so it cannot include the trailing newline. We should keep this as a bug because uv currently accepts the malformed backend metadata and silently produces an incomplete lockfile instead of reporting it.

## Classification

This is a bug. The lockfile is incomplete even though the command reports success, and the supplied log plus the isolated reproduction establish why the dependency groups and project dependencies diverge. The upstream Hatchling regression explains the malformed input, but uv's silent acceptance still produces incorrect user-visible behavior.

This is not a duplicate. No open or closed uv issue or pull request was found for the same raw-version newline mechanism or for runtime dependencies disappearing while dependency groups remain.

## Related

- pypa/hatch#2398 — merged pull request, “Fix version metadata to preserve leading zeros in CalVer.” It changed Hatchling to write the original dynamic version string into core metadata. In this report, the original value includes the newline captured by `(?P<version>[^']+)`, which places a blank line before `Requires-Dist` and directly causes those fields to be ignored.

## Search coverage

Searches covered open and closed uv issues and open, closed, and merged uv pull requests. Literal queries included `project.dependencies`, `dependency-groups`, missing or removed direct dependencies, `uv lock`, `upgrade-package`, `DynamicField("version")`, `No static pyproject.toml available`, `Requires-Dist`, Hatchling, and uv 0.12.16. Conceptual queries covered stale or malformed build metadata, dynamic versions, dependencies omitted from lockfiles, backend metadata validation, and dependency changes not being picked up. Fix-oriented review covered the uv 0.12.16 release changes and recent resolver, lockfile, metadata, and local-dependency pull requests. The Hatchling 1.32.3 release and its originating pull request were followed after the verbose log identified that backend version.

astral-sh/uv#6712 was inspected because uv 0.3.5 failed to notice changed dependencies for a dynamic-version editable project. It was ruled out because that regression reused stale `.egg-info`; astral-sh/uv#21824 prepares fresh Hatchling metadata, and clearing the cache does not address the malformed output. astral-sh/uv#11047 was also ruled out because it concerns a dynamic version intermittently appearing in the lockfile after a cached build, not missing `Requires-Dist` fields. astral-sh/uv#10776 uses Hatchling with a dynamic version, but its confirmed trigger is recursive self-referential extras and its symptom is a perpetually stale lockfile, so it is not the same failure.
