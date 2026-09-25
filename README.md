# nushell's venv activation script doesn't work

Issue: astral-sh/uv#21978

Classification: bug

## Summary

The reported failure is reproducible for a relocatable virtual environment. Nushell rejects line 72
of the generated `activate.nu` while parsing `overlay use`:

```nu
let virtual_env = (path self | path dirname | path dirname)
```

Nushell reports that `path self` can only run during parse time and suggests assigning its output to
a constant. Although the report omits the virtual-environment creation command, uv emits this exact
line only for `uv venv --relocatable`, so that was the targeted fixture. With uv 0.12.18, CPython
3.12.3, and Nushell 0.115.1 on Linux x86_64, the relocatable script failed with the same diagnostic
and exit status 1. A non-relocatable control created by the same uv version activated successfully,
printed its environment path, and exited with status 0.

Relocatable Nushell activation was implemented by astral-sh/uv#17036 to satisfy
astral-sh/uv#16973, so successful activation after moving an environment is intended behavior.

No existing issue or pull request tracks this exact parse-time failure. The existing Rust test only
checks that the problematic text is generated, while the Nushell integration workflow executes a
standard, non-relocatable script and does not reach this branch. astral-sh/uv#15294 tracks the
broader need for activation-script integration coverage.

## Reproduction

Outcome: **reproducible**.

The reproduction used a temporary directory for the virtual environments, uv tool installation,
configuration, and caches. The installed uv 0.12.13 was used to launch the reporter's exact uv
version through `uvx`; the resulting uv reported `uv 0.12.18 (x86_64-unknown-linux-gnu)`. The other
relevant versions were Nushell 0.115.1, CPython 3.12.3 at `/usr/bin/python3`, and Linux x86_64.

Minimal reproduction (with `repro_root` set to a fresh temporary directory and Nushell 0.115.1 on
`PATH`):

```bash
export UV_CACHE_DIR="$repro_root/uv-cache"
export UV_TOOL_DIR="$repro_root/uv-tools"
export UV_TOOL_BIN_DIR="$repro_root/uv-tool-bin"
export UV_PYTHON_DOWNLOADS=never
uvx --from uv==0.12.18 uv venv --python /usr/bin/python3 --relocatable "$repro_root/relocatable"
nu --no-config-file -c "overlay use '$repro_root/relocatable/bin/activate.nu'; print \$env.VIRTUAL_ENV"
```

The generated line was:

```nu
let virtual_env = (path self | path dirname | path dirname)
```

The `nu` command exited with status 1 and produced the reported `this command can only run during
parse-time` error at `activate.nu:72:24`, with the same suggestion to assign the output to a
constant. As a control, omitting `--relocatable` generated an absolute path at line 72; that script
activated successfully, printed the expected environment path, and exited with status 0.

Existing test coverage does not exercise the failure. In `crates/uv/tests/python/venv.rs`,
`verify_pyvenv_cfg_relocatable` asserts that `activate.nu` contains the failing `let virtual_env =
(path self | path dirname | path dirname)` text but never runs Nushell. The Nushell job in
`.github/workflows/test-integration.yml` does run `overlay use`, but its `Create venv` step calls
`./uv venv` without `--relocatable`, so it tests only the successful control branch.

## Draft response

Thanks for the report. uv's source confirms that this `path self` expression is emitted
specifically for relocatable Nushell activation scripts; astral-sh/uv#17036 introduced it so those
scripts could locate the environment after it was moved. I reproduced the same parser error with uv
0.12.18 and Nushell 0.115.1 by creating the environment with `uv venv --relocatable`; a standard
environment activated successfully. Our Rust test checks only the generated text, and the current
Nushell integration check creates a non-relocatable environment, so neither catches this failure.
Broader activation-script coverage is tracked in astral-sh/uv#15294.

Could you still share the output of `nu --version` and confirm that the environment was created with
`--relocatable`? The bug itself is reproduced, but that information would identify the reporter's
exact compatibility boundary.

## Classification

`bug` is the appropriate classification. A targeted execution with the reported uv version produces
the exact diagnostic, while the non-relocatable control works, and relocatable Nushell activation is
explicitly intended behavior. The reporter's Nushell version remains unknown, but current Nushell
0.115.1 is affected. It is not a duplicate: no open issue or pull request was found for this
parse-time failure. The historical work enabled this code path; it does not already track the new
failure.

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
