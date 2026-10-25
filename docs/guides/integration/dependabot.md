---
title: Using uv with Dependabot
description: Use uv with the Dependabot dependency bot.
---

# Dependabot

Update dependencies regularly to reduce exposure to vulnerabilities and limit incompatibilities.
Regular updates also prevent complex upgrades from outdated versions.

Dependabot supports uv, but some use cases do not work yet. See
[astral-sh/uv#2512](https://github.com/astral-sh/uv/issues/2512) for updates.

To update `uv.lock` files, add the uv `package-ecosystem` to the `updates` list in `dependabot.yml`:

```yaml title="dependabot.yml"
version: 2

updates:
  - package-ecosystem: "uv"
    directory: "/"
    schedule:
      interval: "weekly"
```

## Dependency cooldown

If you use the [`exclude-newer`](../../reference/settings.md#exclude-newer) option, also configure
the equivalent
[`cooldown`](https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-options-reference#cooldown-)
option in Dependabot. This prevents pull requests with dependencies that uv cannot lock.

If `exclude-newer` is set to `1 week`, use this configuration:

```yaml title="dependabot.yml"
version: 2

updates:
  - package-ecosystem: "uv"
    directory: "/"
    schedule:
      interval: "weekly"
    cooldown:
      default-days: 7
```
