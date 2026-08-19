# Index username stripped from url in `uv-receipt.toml` preventing tool upgrades

Issue: astral-sh/uv#21216

Classification: duplicate

## Summary

The reporter installs a tool from a private index configured through the legacy `index-url`
setting in user-level `uv.toml`. The URL contains a username but no password because
`keyring-provider = "subprocess"` supplies the secret. The installed tool's `uv-receipt.toml`
retains the index URL but removes the username. On a later `uv tool upgrade`, the receipt's
credential-free `index-url` takes precedence over the user configuration. The subprocess keyring
lookup is skipped because no username is available, the index returns 401 responses, and uv
misleadingly reports `Nothing to upgrade`.

This is a concrete username-only/keyring reproduction of the private-index tool-upgrade
configuration problem already tracked by astral-sh/uv#8523. Historical fixes cover matching named
indexes and embedded or environment credentials, but not this legacy `index-url` plus subprocess
keyring combination.

## Draft response

Thanks for the detailed reproduction. This is the same underlying issue as astral-sh/uv#8523.
`uv tool upgrade` prefers the `index-url` recorded in the receipt over the current user
configuration, while receipt serialization removes URL credentials, including a username without
a password. In this case that leaves subprocess keyring without the username it needs, and the
failed index queries are then reported as `Nothing to upgrade`.

The equivalent named `[[index]]` credential-transfer path was fixed in astral-sh/uv#14858, and
astral-sh/uv#18246 made invalid embedded credentials fail loudly, but that fix explicitly did not
cover keyring. Let's keep the remaining `index-url`/keyring work in astral-sh/uv#8523; your
reproduction is a useful concrete case for that issue.

## Classification

Duplicate of astral-sh/uv#8523. Both reports concern `uv tool upgrade` using the private-index
configuration persisted in the tool receipt instead of usable, current authentication from the
user configuration. astral-sh/uv#21216 narrows the trigger to a username-only URL used with the
subprocess keyring and demonstrates the resulting 401-to-no-op behavior, but the underlying place
to centralize the fix is the same open issue.

This is not a regression of the cases covered by the prior merged fixes. astral-sh/uv#14858 handles
credentials from all duplicated named/default index definitions, whereas the scalar legacy
`index-url` from the receipt wins during option combination. astral-sh/uv#18246 promotes the saved
authentication policy when credentials are embedded or supplied through environment variables;
its pull request description explicitly excludes keyring-provided credentials.

## Related

- astral-sh/uv#8523 — Open canonical issue. It reports private-index tool upgrades using receipt
  settings instead of refreshed credentials in user-level `uv.toml`, proposes matching the current
  index by credential-free domain/URL, and uses uninstall/reinstall as a workaround. Those are the
  same command, subsystem, configuration-precedence problem, and requested outcome as here.
- astral-sh/uv#14806 — Closed historical near-match. Maintainers confirmed two problems: index
  credentials were removed from the receipt, and matching credentials from `uv.toml` were not
  transferred to `uv tool upgrade`.
- astral-sh/uv#14855 — Merged pull request that intentionally changed receipt serialization from an
  unusable redacted password to removal of all URL credentials. It establishes that persisting the
  original credential-bearing URL is not the intended general solution; later commands need to
  recover authentication safely.
- astral-sh/uv#14858 — Merged fix for astral-sh/uv#14806. It considers all known named/default index
  definitions when caching credentials, including an authenticated configuration entry shadowed by
  a credential-free receipt entry. It does not fix the scalar legacy `index-url` precedence in this
  report.
- astral-sh/uv#18120 — Closed issue matching the misleading `Nothing to upgrade` result when a
  private index cannot be authenticated even though a newer tool version exists.
- astral-sh/uv#18246 — Merged fix for astral-sh/uv#18120. It promotes `authenticate = "always"` in
  receipts when embedded or environment credentials were present, so an unauthenticated upgrade
  fails instead of silently succeeding. The pull request explicitly says keyring credentials are
  not covered.

## Supporting evidence

Current source behavior supports the report:

- `IndexUrl` serialization calls `without_credentials`, which removes both the username and
  password before an index URL is written to a receipt.
- `uv tool upgrade` combines settings in the documented order CLI, then receipt, then user
  configuration. A receipt's scalar `index-url` therefore prevents the equivalent scalar setting
  from user-level `uv.toml` from supplying the omitted username.
- The report's debug log shows the observable consequence: no username is available, subprocess
  keyring is skipped, index requests return 401, and resolution retains the installed versions.

Searches covered exact terms (`uv-receipt.toml`, `index-url`, `tool upgrade`, `Nothing to upgrade`,
401, username, and the keyring skip message), conceptual terms (private-index authentication,
credential stripping/persistence, receipt precedence, refreshed credentials, and keyring), and
historical fix searches across open and closed issues and open, closed, and merged pull requests.
astral-sh/uv#16597 was inspected because it also combines tool receipt credential removal with a
later private-index failure, but it concerns `uv tool run` with a full embedded token rather than
the upgrade/configuration-precedence path. astral-sh/uv#15034 was also inspected because it requests
preserving a username while dropping a secret, but it concerns Git source serialization during
`uv add`, not package-index tool receipts.
