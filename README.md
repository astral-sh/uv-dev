# Bash prompt values can change meaning under older compatibility modes (POSIX)

Issue: astral-sh/uv#23058

Classification: bug

## Summary

The reporter describes command substitution when sourcing a virtual environment's Bash/Zsh activation script in uv 0.13.6 on Fedora Linux. The supplied reproduction uses `uv venv --prompt` with a value containing `$(...)`. The body also attributes immediate execution to environment directory names. The title mentions older Bash compatibility modes and POSIX mode, although the supplied reproduction does not enable either.

No uv duplicate found. astral-sh/uv#9424 and astral-sh/uv#8984 document earlier path quoting; astral-sh/uv#23054 concerns a different prompt serialization defect. Upstream pypa/virtualenv#3418 addresses the compatibility-sensitive prompt rendering described by the title.

The current source confirms unsafe interpolation of an explicit prompt, and a harmless Bash template check confirms evaluation without compatibility mode or interactive prompt rendering. A separate directory-derived check establishes a timing distinction: the name remains literal during activation, then undergoes command substitution when Bash renders PS1.

The reporter says the security team considered this outside uv's vulnerability threat model. This handoff classifies the established behavior as a correctness bug; it does not independently establish CVE applicability.

## Draft response

Your `--prompt` example exposes a bug in the current activation template: the value is inserted unescaped into double-quoted shell code, allowing command substitution while sourcing it, including in ordinary Bash. The path-quoting fix in astral-sh/uv#8984, discussed in astral-sh/uv#9424, does not cover this value.

Directory-derived prompts differ: `basename` preserves the text during activation, but Bash can evaluate it when rendering `PS1`. The next step is to fix both script-generation quoting and prompt rendering, with regression coverage for explicit prompts, directory names, and Bash compatibility/POSIX settings. Upstream pypa/virtualenv#3418 is relevant to rendering, but does not by itself address uv's source-time interpolation.

## Classification

The current generator inserts explicit prompt text unescaped into double-quoted activation code. A harmless check of the checkout's template in Bash 5.2 confirmed command substitution during activation; a separate check confirmed directory-derived text is evaluated when rendering PS1. These are correctness defects regardless of security classification. The historical path-quoting fix did not cover prompt interpolation, so this is not an established regression of that fix, and no existing uv issue or PR was found that tracks the same problem.

The title's compatibility-specific behavior and the body's source-time reproduction need separate consideration. The referenced upstream change concerns prompt redraw and Bash settings. It is useful implementation context, but neither an upstream merge nor the old uv path-quoting fix establishes that this prompt issue was previously fixed in uv.

## Related

- astral-sh/uv#9424 — Please comment on CVE-2024-53899 and virtualenv activation scripts (issue, closed). Historical activation-script command-injection discussion. Maintainers pointed to astral-sh/uv#8984 for environment-path quoting and treated quoting defects as bugs. That fix did not escape prompt values, so this is related context rather than a duplicate or evidence of a regression.

- astral-sh/uv#8984 — Allow apostrophe in venv name (pull request, merged). Merged on 2024-11-15 to fix astral-sh/uv#8947. The diff escapes VIRTUAL_ENV_DIR for single-quoted shell assignments; it leaves VIRTUAL_PROMPT interpolation unchanged. The current prompt bug is outside that fix's scope.

- astral-sh/uv#23054 — Venv configuration readback can reinterpret path boundaries (issue, open). The same reporter identifies another uv 0.13.6 problem involving --prompt and directory names. It concerns embedded newlines creating pyvenv.cfg directives and changing isolation, whereas this report concerns shell evaluation in activation scripts; the output formats and required fixes differ.

- pypa/virtualenv#3418 — 🐛 fix(bash): preserve literal prompts in supported shells (pull request, merged). Upstream PR corresponding to the reporter's linked commit, merged on 2026-10-17. It addresses PS1 rendering under Bash compatibility levels, promptvars, and POSIX mode. It informs a rendering fix, but does not establish that uv's separate source-time prompt interpolation was fixed.

## Supporting evidence

Source inspected at checkout commit `39a02a0d9085f14dc3d5aa8655ac010a1fd377f8`:

- `crates/uv-virtualenv/src/lib.rs:71` maps an explicit `--prompt` to `Prompt::Static`, `--prompt .` to the current directory name, and no prompt option to `Prompt::None`.
- `crates/uv-virtualenv/src/virtualenv.rs:97` obtains the explicit/current-directory prompt value before script generation.
- `crates/uv-virtualenv/src/virtualenv.rs:494` escapes the environment path for single quotes. At line 511, prompt escaping is special-cased only for Xonsh; other templates receive the original prompt. Line 536 substitutes that value directly into `{{ VIRTUAL_PROMPT }}`.
- `crates/uv-virtualenv/src/activator/activate:106` embeds that placeholder in both a double-quoted test and a double-quoted assignment. Shell substitution can therefore occur before any prompt is drawn.
- `crates/uv-virtualenv/src/activator/activate:109` obtains the fallback prompt through `basename "$VIRTUAL_ENV"`. Its output is not rescanned as shell source at assignment time. Line 121 inserts its value directly into `PS1`, where Bash prompt expansion can evaluate it.
- The diff and comments for astral-sh/uv#8984 confirm its change was to `VIRTUAL_ENV_DIR`, not the prompt. Its 2024 merge predates the reported 0.13.6 behavior but is not evidence that prompt handling regressed.
- The reporter's upstream commit `5f48d711731311428aebfae61d46957762155b22` corresponds to pypa/virtualenv#3418. Its diff handles prompt rendering for Bash/Zsh, Bash compatibility levels, `promptvars`, and POSIX mode. It does not modify uv's template replacement code.

## Verification and limits

Two independent checks rendered the checked-in activation template in memory using the same relevant placeholder replacements as the generator, then ran it through a clean, noninteractive Bash 5.2.21 subprocess. The substitutions used only `printf`; no marker files or virtual environments were created.

For an explicit prompt `x$(printf UV_PROMPT_SUBSTITUTED)y`, reading `VIRTUAL_ENV_PROMPT` after activation produced:

```text
prompt=xUV_PROMPT_SUBSTITUTEDy
```

With an empty explicit prompt and an environment path whose final component was `x$(printf UV_DIRECTORY_SUBSTITUTED)y`, reading the value and then rendering the prompt with Bash's `${PS1@P}` produced:

```text
after_activation=x$(printf UV_DIRECTORY_SUBSTITUTED)y
rendered_prompt=(xUV_DIRECTORY_SUBSTITUTEDy) 
```

These checks validate the source mechanism, not a complete uv CLI reproduction. No uv binary or Zsh executable was available on PATH. Zsh runtime behavior, older Bash binaries, and compatibility/POSIX option combinations were not exercised. The reported Fedora environment and CVE association were not independently reproduced.

Nearby coverage was inspected: `create_venv_apostrophe` in `crates/uv/tests/python/venv.rs:2132` exercises environment-path quoting; `verify_pyvenv_cfg_relocatable` at line 1836 checks generated files, including Xonsh prompt encoding. Neither covers Bash command substitution from a prompt.

## Suggested follow-up

Fix both the shell source representation of prompt values and their later prompt rendering. Add focused regression coverage for explicit prompts and directory-derived prompts, keeping activation-time evaluation separate from redraw behavior. Check shell-specific behavior and Bash compatibility/POSIX settings when choosing the rendering approach. Use the existing virtual-environment integration-test module and the appropriate shell/platform/feature gates.

## Search coverage and excluded candidates

Used authenticated gh searches across astral-sh/uv open and closed issues and open, closed, and merged PRs. Searched source-time substitution and directory-derived prompts separately from the title's compatibility claim: CVE-2026-113723, command substitution, PS1, VIRTUAL_ENV_PROMPT, BASH_COMPAT, promptvars, Bash/POSIX, backticks, dollars, and backslashes. Conceptual searches covered activation injection, command execution, prompt escaping, literal prompts, directory quoting, and special characters, including area:environments. Fix-oriented searches covered activation, prompt, and escaping PRs; also inspected 1,000 recent PR titles and followed astral-sh/uv#9424 through astral-sh/uv#8947, astral-sh/uv#8984, and astral-sh/uv#9466, plus the reported upstream commit and pypa/virtualenv#3418. Inspected comments and ruled out astral-sh/uv#13456 (prompt formatting), astral-sh/uv#12662 (SCRIPT_PATH leakage), astral-sh/uv#10498 (unset OSTYPE), and astral-sh/uv#5950 (printed Windows activation paths). No same-problem uv tracker or prompt fix was found.

Additional inspected candidates were excluded: astral-sh/uv#10372 concerns pyenv shims taking precedence after activation; astral-sh/uv#2668 concerns a Powerlevel10k formatting interaction; astral-sh/uv#7467 requests colored prompts. The batch-path proposal astral-sh/uv#9466 was closed without merging, so the historical comment describing it as a fix must not be read as evidence of a released fix.

Searches used both literal report terms and alternative terminology without restricting to Fedora or uv 0.13.6. Some requests briefly hit GitHub search rate limits; the state-specific activation/escaping searches and key conceptual searches were retried successfully.

## Scope of changes

Only this temporary issue-context README was authored. No checkout files, GitHub issues, pull requests, comments, or labels were changed.
