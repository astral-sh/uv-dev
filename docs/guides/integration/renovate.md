---
title: Using uv with Renovate
description: Use uv with the Renovate dependency bot.
---

# Renovate

Update dependencies regularly to reduce exposure to vulnerabilities and limit incompatibilities.
Regular updates also prevent complex upgrades from outdated versions.

[Renovate](https://github.com/renovatebot/renovate) supports uv.

## `uv.lock` output

A `uv.lock` file tells Renovate that uv manages the project dependencies. Renovate suggests updates
to [project dependencies](../../concepts/projects/dependencies.md#project-dependencies),
[optional dependencies](../../concepts/projects/dependencies.md#optional-dependencies), and
[development dependencies](../../concepts/projects/dependencies.md#development-dependencies).
Renovate updates both `pyproject.toml` and `uv.lock`.

To refresh the lockfile regularly and update transitive dependencies, enable the
[`lockFileMaintenance`](https://docs.renovatebot.com/configuration-options/#lockfilemaintenance)
option:

```jsx title="renovate.json5"
{
  $schema: "https://docs.renovatebot.com/renovate-schema.json",
  lockFileMaintenance: {
    enabled: true,
  },
}
```

## Inline script metadata

Renovate updates dependencies defined with
[inline script metadata](../scripts.md#declaring-script-dependencies).

Renovate cannot automatically detect Python files that use inline script metadata. Specify their
locations with
[`managerFilePatterns`](https://docs.renovatebot.com/configuration-options/#managerfilepatterns), as
shown here:

```jsx title="renovate.json5"
{
  $schema: "https://docs.renovatebot.com/renovate-schema.json",
  pep723: {
    managerFilePatterns: [
      "docs/build.py",
      "scripts/**/*.py",
    ],
  },
}
```

!!! note

    Renovate does not support updating the lockfile associated with a script
    (https://github.com/renovatebot/renovate/issues/33591). If you use this feature, update the
    script lockfile manually.

## Dependency cooldown

If you use the [`exclude-newer`](../../reference/settings.md#exclude-newer) option, also configure
the equivalent
[`minimumReleaseAge`](https://docs.renovatebot.com/configuration-options/#minimumreleaseage) option
in Renovate. This prevents pull requests with dependencies that uv cannot lock.

If `exclude-newer` is set to `1 week`, use this configuration:

```jsx title="renovate.json5"
{
  $schema: "https://docs.renovatebot.com/renovate-schema.json",

  // Enable only for PyPI.
  packageRules: [
    {
      matchDatasources: ["pypi"],
      minimumReleaseAge: "1 week",
    },
  ],

  // Or enable for every ecosystem.
  minimumReleaseAge: "1 week",
}
```
