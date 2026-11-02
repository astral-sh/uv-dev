# Bash prompt values can change meaning under older compatibility modes (POSIX)

Issue: astral-sh/uv#23058

Classification: bug

## Summary

The explicit `uv venv --prompt` example is **reproducible** with the installed uv 0.13.6 executable. On Ubuntu 24.04.5, x86_64 Linux, CPython 3.12.3, and Bash 5.2.21, command substitution in the prompt runs while sourcing the generated activation script, before any interactive prompt is drawn. The same behavior occurs in ordinary Bash, Bash POSIX mode, and Bash with `compat42` enabled. Older compatibility settings are not necessary for the body's example.

The body's claim about environment directory names needs a timing correction. With no explicit `--prompt`, a directory name containing `$(...)` remains literal during activation. Bash subsequently evaluates it during prompt expansion, confirmed with `${PS1@P}`. This distinction also holds in the tested POSIX and compatibility modes.

The report specifies Fedora Linux and uv 0.13.6, with any Python version. Fedora, Zsh, and older Bash binaries were not exercised. The reporter associates the behavior with CVE-2026-113723 and says the security team considered it outside uv's vulnerability threat model; this reproduction does not establish CVE applicability.

## Classification

Reproducible prompt-quoting correctness bug. The generated script embeds explicit prompt text directly into double-quoted shell source, and the CLI reproduction confirms its evaluation during activation. Directory-derived prompt text follows a separate path and is evaluated during Bash prompt expansion instead.

The title's compatibility-specific wording should not obscure the ordinary-Bash reproduction in the body. No upgrade regression or last known-good uv version is established. The historical environment-path quoting fix in astral-sh/uv#8984 does not cover explicit prompt interpolation. Prior related-issue research found no same-problem uv tracker or prompt fix.

## Reproduction

Outcome: **reproducible** for the explicit-prompt Bash example, including POSIX and `compat42` modes. The default directory-derived case reproduces substitution at prompt expansion, not at source time.

### Environment and isolation

- Installed executable: `/opt/hostedtoolcache/uv/0.13.6/x86_64/uv`, resolved from `PATH`; `uv --version` reports `uv 0.13.6 (x86_64-unknown-linux-gnu)`.
- Host: Ubuntu 24.04.5, Linux x86_64.
- Shell: GNU Bash 5.2.21(1)-release; Python: `/usr/bin/python3`, CPython 3.12.3.
- All environments, marker files, temporary files, and caches were placed in `/tmp/uv-23058-ysiw1cc0`. Command arguments, exit codes, stdout, and stderr are retained in its `results.json`.
- Commands ran with a clean subprocess environment, `PATH=/usr/bin:/bin`, temporary `TMPDIR`, `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR`, and XDG cache/config/data directories, and `UV_PYTHON_DOWNLOADS=never`. The uv executable was invoked by its resolved absolute path. `--offline --no-config` and an explicit interpreter avoided network access, project/user configuration, and Python downloads. Bash used `--noprofile --norc`.

### Explicit prompt: activation-time evaluation

Minimal equivalent commands, run inside a fresh temporary directory with a temporary uv cache:

```bash
uv venv --offline --no-config --python /usr/bin/python3 \
  --prompt 'x$(printf UV_PROMPT_SUBSTITUTED)y' explicit
bash --noprofile --norc -c \
  'source explicit/bin/activate; printf "prompt=%s\n" "$VIRTUAL_ENV_PROMPT"'
```

Expected literal value: `prompt=x$(printf UV_PROMPT_SUBSTITUTED)y`. Observed output, with exit code 0:

```text
prompt=xUV_PROMPT_SUBSTITUTEDy
```

There is no interactive shell or prompt expansion in this command. The generated script contains:

```bash
if [ "xx$(printf UV_PROMPT_SUBSTITUTED)y" != x ] ; then
    VIRTUAL_ENV_PROMPT="x$(printf UV_PROMPT_SUBSTITUTED)y"
```

A separate marker check confirms a side effect at source time using only a file in the temporary working directory:

```bash
uv venv --offline --no-config --python /usr/bin/python3 \
  --prompt 'x$(touch prompt-executed)y' marker
bash --noprofile --norc -c \
  'source marker/bin/activate; printf "prompt=%s\n" "$VIRTUAL_ENV_PROMPT"'
```

`prompt-executed` was absent after environment creation and present after sourcing. The shell printed `prompt=xy` and exited 0. No prompt redraw was requested.

Two targeted settings checks against the `explicit` environment also exited 0 and printed `prompt=xUV_PROMPT_SUBSTITUTEDy`:

```bash
bash --noprofile --norc --posix -c \
  'source explicit/bin/activate; printf "prompt=%s\n" "$VIRTUAL_ENV_PROMPT"'
bash --noprofile --norc -c \
  'shopt -s compat42; source explicit/bin/activate; printf "prompt=%s\n" "$VIRTUAL_ENV_PROMPT"'
```

### Directory-derived prompt: evaluation during prompt expansion

```bash
uv venv --offline --no-config --python /usr/bin/python3 \
  'x$(printf UV_DIRECTORY_SUBSTITUTED)y'
bash --noprofile --norc -c \
  'source "$1"; printf "after_activation=%s\nrendered_prompt=%s\n" "$VIRTUAL_ENV_PROMPT" "${PS1@P}"' \
  bash 'x$(printf UV_DIRECTORY_SUBSTITUTED)y/bin/activate'
```

Observed output, exit code 0:

```text
after_activation=x$(printf UV_DIRECTORY_SUBSTITUTED)y
rendered_prompt=(xUV_DIRECTORY_SUBSTITUTEDy) 
```

`${PS1@P}` explicitly applies Bash prompt expansion without requiring an interactive terminal. The same command was separately exercised with `--posix`, and with `shopt -s compat42` before activation; both produced the same output. The directory-derived value is literal after sourcing, so the body's generalization that both prompt sources execute immediately is not accurate for this case.

### Existing test coverage and limits

Searched `crates/uv/tests/` and `crates/uv-client/tests/it/` for prompt substitution, `VIRTUAL_ENV_PROMPT`, `PS1`, `BASH_COMPAT`, and `promptvars`. Read the relevant setup and assertions in `crates/uv/tests/python/venv.rs`:

- `create_venv_apostrophe`: creates `Testing's`, activates it, runs Python, and compares `sys.prefix` with the environment path. It covers apostrophe quoting in environment paths, not command substitution in prompts. Linux-only.
- `verify_pyvenv_cfg_relocatable`: uses a prompt containing Unicode, quotes, a backslash, and a newline; checks relocatable boilerplate and the encoded Xonsh prompt. It does not source the Bash script or assert literal Bash prompt behavior.
- `activate_non_relocatable_venv_ksh`: uses a POSIX shell with Bash/Zsh version variables unset and a synthetic `KSH_VERSION` to check activation with a read-only `SCRIPT_PATH`. It does not test prompt interpolation. Unix-only.

The module is gated by `test-python` in `crates/uv/tests/python/main.rs`. No existing test covering the reported Bash substitution was found in these searches. No repository tests were added or run; verification used the installed CLI and its generated scripts.

Zsh was unavailable. These results do not verify Zsh behavior, the exact Fedora environment, an actual older Bash executable, every shell option combination, or a separate compatibility-only regression. They confirm the Bash example in the body with the reported uv version. The earlier template-only checks are superseded by this complete CLI reproduction.

## Related

- astral-sh/uv#9424 — Please comment on CVE-2024-53899 and virtualenv activation scripts (issue, closed). Historical activation-script command-injection discussion. Maintainers pointed to astral-sh/uv#8984 for environment-path quoting and treated quoting defects as bugs. That fix did not escape prompt values, so this is related context rather than a duplicate or evidence of a regression.

- astral-sh/uv#8984 — Allow apostrophe in venv name (pull request, merged). Merged on 2024-11-15 to fix astral-sh/uv#8947. The diff escapes VIRTUAL_ENV_DIR for single-quoted shell assignments; it leaves VIRTUAL_PROMPT interpolation unchanged. The current prompt bug is outside that fix's scope.

- astral-sh/uv#23054 — Venv configuration readback can reinterpret path boundaries (issue, open). The same reporter identifies another uv 0.13.6 problem involving --prompt and directory names. It concerns embedded newlines creating pyvenv.cfg directives and changing isolation, whereas this report concerns shell evaluation in activation scripts; the output formats and required fixes differ.

- pypa/virtualenv#3418 — 🐛 fix(bash): preserve literal prompts in supported shells (pull request, merged). Upstream PR corresponding to the reporter's linked commit, merged on 2026-10-17. It addresses PS1 rendering under Bash compatibility levels, promptvars, and POSIX mode. It informs a rendering fix, but does not establish that uv's separate source-time prompt interpolation was fixed.


## Supporting implementation evidence

Source inspected at checkout commit `39a02a0d9085f14dc3d5aa8655ac010a1fd377f8`:

- `crates/uv-virtualenv/src/lib.rs`: `Prompt::from_args` maps explicit text to `Prompt::Static`, `--prompt .` to `Prompt::CurrentDirectoryName`, and no option to `Prompt::None`.
- `crates/uv-virtualenv/src/virtualenv.rs`: script generation escapes environment paths for single-quoted shell assignments, but only Xonsh receives special handling for prompt text. Other templates receive the prompt directly through `{{ VIRTUAL_PROMPT }}` replacement.
- `crates/uv-virtualenv/src/activator/activate`: the explicit prompt appears in a double-quoted test and assignment. The fallback uses `basename "$VIRTUAL_ENV"`, whose output is not rescanned as shell source at assignment time. `PS1="(${VIRTUAL_ENV_PROMPT}) ${PS1-}"` then incorporates the value into the prompt.

The installed executable's generated scripts match these relevant template fragments. The observations above establish the behavior independently of source inspection. The reporter's upstream commit `5f48d711731311428aebfae61d46957762155b22`, associated with pypa/virtualenv#3418, is context for prompt rendering; it does not modify uv's generator.

## Suggested follow-up

Handle both shell-source quoting for explicit prompts and literal prompt rendering for directory-derived values. Keep source-time evaluation separate from redraw behavior in focused integration coverage. Check shell-specific behavior and Bash compatibility/POSIX settings when choosing a rendering approach. Use the existing virtual-environment integration-test module with its `test-python` gate and appropriate shell/platform conditions.

## Search coverage and excluded candidates

The following related-issue research is retained from the existing issue context; no new GitHub searches or changes were made during CLI reproduction.

Used authenticated gh searches across astral-sh/uv open and closed issues and open, closed, and merged PRs. Searched source-time substitution and directory-derived prompts separately from the title's compatibility claim: CVE-2026-113723, command substitution, PS1, VIRTUAL_ENV_PROMPT, BASH_COMPAT, promptvars, Bash/POSIX, backticks, dollars, and backslashes. Conceptual searches covered activation injection, command execution, prompt escaping, literal prompts, directory quoting, and special characters, including area:environments. Fix-oriented searches covered activation, prompt, and escaping PRs; also inspected 1,000 recent PR titles and followed astral-sh/uv#9424 through astral-sh/uv#8947, astral-sh/uv#8984, and astral-sh/uv#9466, plus the reported upstream commit and pypa/virtualenv#3418. Inspected comments and ruled out astral-sh/uv#13456 (prompt formatting), astral-sh/uv#12662 (SCRIPT_PATH leakage), astral-sh/uv#10498 (unset OSTYPE), and astral-sh/uv#5950 (printed Windows activation paths). No same-problem uv tracker or prompt fix was found.

Additional inspected candidates were excluded: astral-sh/uv#10372 concerns pyenv shims taking precedence after activation; astral-sh/uv#2668 concerns a Powerlevel10k formatting interaction; astral-sh/uv#7467 requests colored prompts. The batch-path proposal astral-sh/uv#9466 was closed without merging, so the historical comment describing it as a fix must not be read as evidence of a released fix.

Searches used both literal report terms and alternative terminology without restricting to Fedora or uv 0.13.6. Some requests briefly hit GitHub search rate limits; the state-specific activation/escaping searches and key conceptual searches were retried successfully.


## Scope of changes

Only this temporary issue-context README was updated; reproduction files and caches were created under `/tmp/uv-23058-ysiw1cc0`. No checkout files or existing user state were modified, and no GitHub issues, pull requests, comments, or labels were changed. The pre-existing checkout changes were left untouched.
