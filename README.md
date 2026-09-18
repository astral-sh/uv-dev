# uv lock fails to pick up direct dependencies

Issue: astral-sh/uv#21824

Classification: bug

## Summary

`uv lock` completes successfully but omits every dependency declared in `[project].dependencies` and their transitive dependencies. Dependencies under `[dependency-groups]` are still resolved. The reported project has a dynamic version supplied by Hatchling, and its build requirements do not constrain the Hatchling version.

The verbose output shows that uv cannot use the project metadata statically because `version` is dynamic, invokes `hatchling.build.prepare_metadata_for_build_editable()`, and selects Hatchling 1.32.3. The prepared project is then added to the solver without either runtime dependency; the dev-group dependencies are added separately from the workspace configuration.

An isolated reproduction confirms the mechanism. With a newline-terminated `VERSION` file and the reported pattern `(?P<version>[^']+)`, Hatchling 1.32.0 emits a valid `Version: 0.1.0` header followed by `Requires-Dist`. Hatchling 1.32.3 emits the captured newline as part of the raw version, producing a blank line immediately after `Version`. Core metadata parsing treats that blank line as the end of the header section, so the later `Requires-Dist` fields are ignored. uv 0.12.16 consequently locks only the dependency group. Pinning the build requirement to `hatchling==1.32.0` restores the runtime dependencies.

Hatchling 1.32.3 was published on 2026-09-17, matching the report's onset. pypa/hatch#2398 introduced the raw-version behavior to preserve leading zeros in CalVer versions.

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

## Supporting evidence

- The report's log selects Hatchling 1.32.3 and calls `prepare_metadata_for_build_editable()` because `[project].dynamic` contains `version`.
- After metadata preparation, the solver adds the project and its dev group, then expands only the dev-group requirements. Neither declared runtime dependency appears.
- A minimal reproduction with uv 0.12.16 and Hatchling 1.32.3 resolves 7 packages and includes the project and pytest group, but omits the two `[project].dependencies` entries.
- The same backend-generated `METADATA` contains two blank lines after `Version: 0.1.0`; the later `Requires-Python` and `Requires-Dist` lines fall outside the parsed header block.
- Pinning the same reproduction to Hatchling 1.32.0 makes uv 0.12.16 resolve 14 packages and records both direct runtime dependencies and their transitive dependencies.
