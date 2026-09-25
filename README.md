# nushell's venv activation script doesn't work

Issue: astral-sh/uv#21978

Classification: bug

## Summary

Running `overlay use .venv/bin/activate.nu` fails when Nushell parses the generated script. The
diagnostic points to line 72:

```nu
let virtual_env = (path self | path dirname | path dirname)
```

Nushell reports that `path self` can only run during parse time and suggests assigning its output to
a constant. The reporter says that changing `let` to `const` makes activation work. The report is
from macOS arm64 with uv 0.12.18 and Python 3.14.7, but it does not include the Nushell version or
the command that created the environment.

The repository source confirms that uv substitutes this exact expression only for a relocatable
`activate.nu`; a non-relocatable script receives a quoted absolute environment path. Relocatable
Nushell activation was implemented by astral-sh/uv#17036 to satisfy astral-sh/uv#16973. That makes
the generated script's failure a bug in intended behavior, although the missing Nushell version and
creation command are needed to establish the precise compatibility boundary.

No existing issue or pull request tracks this exact parse-time failure. The current Nushell
integration workflow installs the latest stable Nushell but creates a standard, non-relocatable
environment, so it does not execute the reported `path self` branch. astral-sh/uv#15294 tracks the
broader need for activation-script integration coverage.

## Draft response

Thanks for the report. uv's source confirms that this `path self` expression is emitted
specifically for relocatable Nushell activation scripts; astral-sh/uv#17036 introduced it so those
scripts could locate the environment after it was moved. Our current Nushell integration check
creates a non-relocatable environment, so it does not exercise this branch; broader
activation-script coverage is tracked in astral-sh/uv#15294.

Could you share the output of `nu --version` and the exact command that created this `.venv`,
including whether `--relocatable` was used? That will let us establish the affected Nushell versions
and reproduce the same generated script. Changing `let` to `const` is a plausible fix based on the
diagnostic, but it still needs validation across the supported Nushell versions.

## Classification

`bug` is the appropriate classification. The source-generated line and Nushell diagnostic match
exactly, and relocatable Nushell activation is explicitly intended behavior. The report therefore
establishes incorrect behavior even though the exact Nushell release that triggers it remains to be
identified. It is not a duplicate: no open issue or pull request was found for this parse-time
failure. The historical work enabled this code path; it does not already track the new failure.

## Related

- astral-sh/uv#17036 — merged pull request, **Fix relocatable nushell activation script**. This is
  the origin of the exact `path self` expression in the failing generated script. It made
  relocatable `activate.nu` derive the environment root dynamically. The new report shows that
  implementation being rejected at parse time; the pull request is historical context, not an
  existing tracker for the regression.
- astral-sh/uv#16973 — closed issue, **Make a genuinely relocatable variant of --relocatable (i.e.
  fix or remove activate.csh/nu)**. This established relocatable Nushell activation as intended
  behavior and was closed by astral-sh/uv#17036. It requested the capability rather than tracking
  the current parser error.
- astral-sh/uv#15294 — open issue, **add integration tests for venv activation scripts**. This is a
  broader, adjacent testing issue. The repository now tests Nushell activation, but that workflow
  creates a non-relocatable venv and therefore misses the `path self` branch involved here.

## Search evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
used the exact diagnostic (`this command can only run during parse-time`), `path self`,
`activate.nu`, and `overlay use`. Conceptual queries covered Nushell virtual-environment activation,
shell compatibility, relocatable environments, and activation-test coverage. Fix-oriented review
included closed reports, merged pull requests, file history for both the Nushell template and its
placeholder substitution, and comments and references on the strongest candidates.

astral-sh/uv#14888 was plausible from its title but concerns a different Nushell 0.106
incompatibility: deprecation of `get -i`, not a parse-time failure at `path self`.
astral-sh/uv#18940 uses the same `overlay use` command but was caused by Windows `Path`/`PATH`
casing and fails on a different expression. Neither is the same underlying problem.
