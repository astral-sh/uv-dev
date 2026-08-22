# uv sync --upgrade does not upgrade a transitive dependency (setuptools)

Issue: astral-sh/uv#21273

Classification: question

## Summary

The reporter is using uv 0.12.5 on Ubuntu with Python 3.14.4. Running
`uv sync --upgrade`, including with `--resolution highest`, leaves setuptools at 81.0.0 in
`uv.lock`. Setuptools is not a direct runtime dependency: it is reached through Pyramid and is also
the project's build-system requirement.

This is expected for the reported dependency graph. Pyramid 2.1 publishes a runtime requirement of
`setuptools<82`, and setuptools 81.0.0 is therefore the highest compatible version. The project has
no direct setuptools requirement that relaxes that transitive upper bound. uv's documented upgrade
behavior remains subject to the dependency graph's constraints, and `--resolution highest` means
the highest *compatible* version rather than the highest published version.

No existing issue or pull request tracks a uv defect matching this exact report. The closest prior
issues cover the same constraint rule and a different transitive-upgrade capability.

## Draft response

Pyramid 2.1 declares `setuptools<82`, so setuptools 81.0.0 is the newest version compatible with
this dependency graph. `--upgrade` and `--resolution highest` still respect package requirements
and therefore cannot select setuptools 83 or newer.

To use a newer setuptools, you'll need a Pyramid release that relaxes that bound. An override is
also possible, but it should only be used after confirming that Pyramid is compatible with the
newer setuptools release.

## Classification

This is a question rather than a bug. The linked project's `uv.lock` contains Pyramid 2.1 and
setuptools 81.0.0, while Pyramid 2.1's published metadata requires `setuptools<82`. The repository
documentation states that upgrades are limited by dependency constraints. Consequently, retaining
81.0.0 is correct resolver behavior, even for a global upgrade and highest resolution.

The report's suggestion that transitive dependencies might be skipped is not supported by the
evidence. The build-system requirement is also not the blocker: the runtime dependency edge from
Pyramid imposes the upper bound.

## Related

- astral-sh/uv#12655 — Closed question where `uv lock --upgrade` could not update packages because
  project constraints excluded newer versions. Maintainers confirmed the same governing rule. The
  difference is that its constraints were direct exact pins, while astral-sh/uv#21273 is blocked by
  Pyramid's transitive upper bound.
- astral-sh/uv#14213 — Closed question about upgrading every transitive dependency of one selected
  parent. Maintainers explained that uv supports upgrading one named transitive package or all
  packages, but not selecting all transitives of one parent. In astral-sh/uv#21273, a global upgrade
  is already requested; the package remains unchanged because of a constraint instead.

No closely matching pull request was found. astral-sh/uv#11784 was inspected because it also
mentions `uv sync --resolution highest`, but that command did not include an upgrade request and
correctly retained lockfile preferences. astral-sh/uv#18178 was also inspected because both a
targeted and global upgrade appeared ineffective, but its cause was a configured
`lowest-direct` resolution mode; the reporter here explicitly tried `highest`. Neither is the same
case.

Searches covered the literal command and package terms (`uv sync --upgrade`, setuptools, Pyramid,
`--resolution highest`, transitive/indirect dependency), conceptual terms (locked versions,
global and targeted upgrades, latest compatible versions, dependency constraints and upper
bounds), and fix-oriented searches across open and closed issues plus open, closed, and merged pull
requests. Searches also removed the package and platform details to look for the underlying
constraint behavior. No version-specific regression or matching fix was found.
