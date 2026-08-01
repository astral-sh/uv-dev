# Cannot opt into prerelease build requirements in uv 0.12

Issue: astral-sh/uv#20875
Classification: bug

## Summary

The closest prior discussion is astral-sh/uv#16494 and astral-sh/uv#16496, which confirm that prerelease policy is fixed for build requirements. astral-sh/uv#19993 fixed only the no-stable-candidate case while introducing uv 0.12's stable-first selection, leaving no way to request prerelease preference for build dependencies.

## Classification

This is a uv 0.12 behavioral regression rather than a duplicate: astral-sh/uv#19993 changed candidate preference to stable-first while build resolutions still ignore every user prerelease policy, a limitation confirmed by astral-sh/uv#8192 and astral-sh/uv#16496. Consequently, a previously effective prerelease build requirement can silently resolve to an older stable version, and the documented opt-in cannot affect that subsystem. No open issue or pull request was found tracking this exact regression.

## Related

- https://github.com/astral-sh/uv/issues/16494 (closed issue): build system requirements not properly resolved
  astral-sh/uv#16494 is the closest historical report: prerelease dependencies in build-system.requires could not be enabled with --prerelease=allow because that policy is not propagated into build resolution. It was closed by astral-sh/uv#19993, but that change only permits prereleases when necessary and does not restore a way to prefer them over an available stable release.
- https://github.com/astral-sh/uv/issues/16496 (closed issue): Build resolutions need different hints
  Maintainer comments in astral-sh/uv#16496 explicitly confirm that build resolutions use fixed prerelease options, ignore --prerelease=allow, and lack equivalent support in uv sync for the build-constraint workaround. It covers both the policy gap and the misleading absence of useful feedback.
- https://github.com/astral-sh/uv/pull/19993 (merged pull request): Support transitive pre-release dependencies
  astral-sh/uv#19993 introduced uv 0.12's stable-first prerelease behavior and added build-resolution coverage for transitive prereleases. Its documented guarantee is that prereleases can be selected when needed, not that they are preferred; combined with fixed build-resolution policy, this caused the reported loss of an effective opt-in path.
- https://github.com/astral-sh/uv/pull/8192 (merged pull request): Don't recommend `--prerelease=allow` during build requirement resolution errors
  astral-sh/uv#8192 is source-backed evidence that --prerelease is deliberately not passed to resolution of build-system.requires. It changed diagnostics rather than adding build-policy propagation, explaining why the options named in the new report have no effect.
