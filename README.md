# Flag '--no-default-groups' has no effect with uv audit

Issue: astral-sh/uv#22089

Classification: bug

## Summary

The reporter finds that `uv audit --no-default-groups` audits the same dependency set as `uv audit`
when `[tool.uv] default-groups = ["dev"]`, while `uv audit --no-group dev` successfully excludes the
group. They reproduced this with uv 0.12.19 on Ubuntu 24.04 under WSL.

The behavior was independently reproduced with the installed uv 0.12.13 on Ubuntu 24.04.5. A
minimal project with one production dependency, a default `dev` group, and a non-default `lint`
group caused both the default invocation and `--no-default-groups` to audit three packages;
`--no-group dev` audited only two.

Repository source explains the observed behavior. `AuditSettings::resolve` passes the parsed
`no_default_groups` value into `DependencyGroups::from_args`, but it also sets the `all_groups`
argument whenever neither `--only-group` nor `--only-dev` is used. The shared dependency-group
selector intentionally gives all-groups precedence over no-default-groups, while explicit
exclusions always win. Consequently, audit's implicit all-groups state makes
`--no-default-groups` ineffective but still allows `--no-group dev` to work. The same plumbing and
help text are present in the uv 0.12.19 source and current main.

No existing issue or pull request tracks this exact defect. The closest history is
astral-sh/uv#18511, which established the intended audit behavior of including everything by
default while allowing broad subset selection. Its implementation supplied the implicit
all-groups state that conflicts with `--no-default-groups`.

## Reproduction

Outcome: **reproducible**.

Environment used for the independent reproduction:

- Ubuntu 24.04.5 LTS, Linux x86_64
- uv 0.12.13 (`x86_64-unknown-linux-gnu`), the installed executable on `PATH`
- CPython 3.12.3

The installed uv is slightly older than the reporter's uv 0.12.19, but it exposes the same audit
option and produces the reported behavior. All fixture files and the uv cache were isolated under
`$RUNNER_TEMP`.

Minimal `pyproject.toml`:

```toml
[project]
name = "audit-groups-repro"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["iniconfig==2.0.0"]

[dependency-groups]
dev = ["typing-extensions==4.10.0"]
lint = ["sniffio==1.3.1"]

[tool.uv]
default-groups = ["dev"]
```

After creating the lockfile with `uv lock`, the targeted commands were:

```console
$ uv audit --locked
Found no known vulnerabilities and no adverse project statuses in 3 packages

$ uv audit --locked --no-default-groups
Found no known vulnerabilities and no adverse project statuses in 3 packages

$ uv audit --locked --no-group dev
Found no known vulnerabilities and no adverse project statuses in 2 packages
```

All three audit commands exited successfully. The first two identical package counts reproduce the
ineffective `--no-default-groups` flag, while the third confirms that explicit group exclusion is
effective in the same fixture.

Existing integration coverage at `crates/uv/tests/build/audit.rs::audit_dependency_groups` verifies
that audit includes all groups by default and that `--no-dev`, `--no-group`, and `--only-group`
change the audited package count. Its fixture does not configure `tool.uv.default-groups`, and it
does not invoke `--no-default-groups`, so it does not cover the reported behavior.

## Draft response

Thanks for the report. This is a bug in the current `uv audit` group-selection plumbing. In uv
0.12.19 and current main, `--no-default-groups` is parsed, but `uv audit` also initializes the
shared selector as though all groups were selected. That all-groups state takes precedence over
`--no-default-groups`, while explicit exclusions such as `--no-group dev` still apply, matching
the behavior you observed.

The intended behavior established in astral-sh/uv#18511 is to audit everything by default while
still allowing subset selection, so the advertised flag should be effective. The next step is to
adjust audit's implicit default selection and add integration coverage for configured default
groups with `--no-default-groups`.

## Classification

This is a bug because an advertised and parsed option cannot produce its documented effect.
Source inspection confirms the correctness problem independently of the reproduction:

- `crates/uv-cli/src/lib.rs` documents `--no-default-groups` as "Don't audit the default dependency
  groups."
- `crates/uv/src/settings.rs` forwards `no_default_groups` but also passes an implicit all-groups
  value whenever no only-group mode is active.
- `crates/uv-configuration/src/dependency_groups.rs` resolves all-groups to `IncludeGroups::All`
  before considering defaults; explicit exclusions remain effective.
- `crates/uv/tests/build/audit.rs::audit_dependency_groups` covers audit's default all-groups
  behavior, `--no-dev`, `--no-group`, and `--only-group`, but not `--no-default-groups`.

This is not a duplicate: no open or closed issue or pull request was found for the same audit
failure. It is also not a regression of astral-sh/uv#10890 or astral-sh/uv#11224. That historical
work intentionally defined precedence when a user explicitly combines `--all-groups` and
`--no-default-groups`; here, the user did not pass `--all-groups`, because audit supplies that state
internally.

## Related

- astral-sh/uv#18511 (merged pull request), "Evaluate extras and groups when determining auditable
  packages" — the closest implementation history. It established that audit includes all extras
  and groups by default while supporting exclusions and subset selection, retained
  `--no-default-groups`, and wired the audit settings to an implicit all-groups value. Its
  discussion explicitly endorses auditing everything by default while allowing subset selection
  broadly.
- astral-sh/uv#11224 (merged pull request), "fix handling of `--all-groups` and
  `--no-default-groups` flags" — adjacent shared-selector history. It intentionally made an
  explicitly supplied `--all-groups` win over `--no-default-groups`. The important difference is
  that astral-sh/uv#22089 does not pass `--all-groups`; audit introduces it implicitly.

## Search and supporting evidence

The GitHub search covered open and closed issues and open, closed, and merged pull requests. Literal
queries included `--no-default-groups`, the exact audit help text, `no effect`, `ignored`,
`ineffective`, and combinations with `uv audit` and `--no-group`. Conceptual queries covered audit
group filtering, group exclusion, subset selection, default groups, and all-groups precedence.
Fix-oriented review covered the audit roadmap, all `area:uv-audit` issues, closed implementation
work, merged integration-test work, and the source history for audit and dependency-group settings.

astral-sh/uv#19973 was a plausible literal match but was ruled out: it concerns the broader design
and asymmetry of extras/groups flags in `uv tree`, not an ineffective flag in `uv audit`.
astral-sh/uv#10890 was also inspected; it concerns combinations of explicit flags in other project
commands and was resolved by astral-sh/uv#11224, so it neither tracks this audit-specific defect nor
represents a previously fixed version of it.
