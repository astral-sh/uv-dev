# Venv configuration readback can reinterpret path boundaries

Issue: astral-sh/uv#23054

Classification: bug

## Summary

The reporter describes uv 0.13.6 on Fedora accepting a prompt containing embedded newlines. The extra lines become entries in `pyvenv.cfg`, including a later `include-system-site-packages = true`, despite the absence of `--system-site-packages`. The reported runtime result is `site.ENABLE_USER_SITE == True` and system/user site-packages appearing in Python's search path. The report also claims a directory-name trigger and cites a virtualenv fix associated with CVE-2026-113715.

The checkout confirms the unvalidated prompt serialization and later-directive parsing. Directory-derived prompt behavior needs a precise distinction: plain `uv venv` uses the current working directory's name, and `--prompt .` selects the same source. An explicit destination without `--prompt` does not put the destination's basename into the configuration's prompt field.

No uv duplicate found. pypa/virtualenv#3426 fixes analogous configuration injection through a different readback path; astral-sh/uv#9424 and astral-sh/uv#8984 concern earlier activation-script quoting bugs.

## Draft response

The source confirms the configuration-writing bug you describe: uv writes `prompt` verbatim after `include-system-site-packages = false`, allowing embedded newlines to introduce another directive. The activation-script fix in astral-sh/uv#8984 does not cover this file.

One detail about directory names: plain `uv venv` and `--prompt .` use the current directory's name as the configured prompt; an explicit destination without `--prompt` leaves that field unset.

The next step is to prevent configuration values from introducing extra directives and add regression coverage for explicit and directory-derived prompts, checking both the generated configuration and Python's site-package isolation.

## Classification

The checkout confirms a correctness defect: write_cfg emits values verbatim, prompt is appended after include-system-site-packages, and uv's configuration parser accepts a later duplicate directive. Explicit prompts and current-directory-derived prompts can therefore introduce configuration entries that were not requested. No same-problem uv tracker or previous fix was found. Earlier activation-script fixes and the upstream virtualenv readback fix address different mechanisms. Runtime reproduction was not performed; source evidence is sufficient for bug classification.

The reporter says the security contact has already assessed this as outside uv's vulnerability threat model. That is reported context, not an independently verified security determination. The classification rests on unintended configuration changes, independently of CVE applicability or vulnerability severity. The upstream fix and earlier shell-quoting fixes do not establish that uv previously fixed this defect.

## Related

- pypa/virtualenv#3426 — 🐛 fix(create): reject ambiguous venv paths (merged). The reporter-linked commit belongs to pypa/virtualenv#3426, merged on 2026-10-17. It fixes line boundaries in destination paths becoming pyvenv.cfg directives through stdlib configuration readback and merging. This is closely analogous, but uv writes prompt values directly and does not use that readback mechanism; the upstream fix does not fix uv.
- astral-sh/uv#9424 — Please comment on CVE-2024-53899 and virtualenv activation scripts (closed). Earlier discussion of virtualenv-derived injection behavior in uv. Maintainer comments identify activation-script path quoting and reference astral-sh/uv#8984 and astral-sh/uv#9466. Those shell-script problems differ from newline-created pyvenv.cfg directives, so this is historical context rather than a duplicate.
- astral-sh/uv#8984 — Allow apostrophe in venv name (merged). Merged on 2024-11-15 to fix astral-sh/uv#8947. Its diff escapes virtual-environment paths inserted into activation scripts and tests an apostrophe-containing path. It does not validate prompt values or change pyvenv.cfg serialization, so the reported behavior is not established as a regression of this fix.

## Supporting evidence

The source inspected is checkout commit `39a02a0d9085f14dc3d5aa8655ac010a1fd377f8`. These findings are from code inspection; the Fedora runtime result remains the reporter's observation.

- `crates/uv-virtualenv/src/virtualenv.rs:55`: `write_cfg` writes each value with `writeln!(f, "{key} = {value}")`. There is no escaping or rejection of embedded line boundaries in this writer.
- `crates/uv-virtualenv/src/virtualenv.rs:97`: static prompt strings are retained verbatim; directory-derived prompts come from `CWD.file_name()`. The path validation at line 108 checks UTF-8 representability, not newlines.
- `crates/uv-virtualenv/src/virtualenv.rs:564`: the system-site option is serialized from the requested boolean. The prompt entry is appended at line 583, and the vector is written at line 597. An embedded directive in the prompt consequently follows the original isolation setting.
- `crates/uv-python-interpreter/src/virtualenv.rs:119`: the reader iterates over lines and partitions each at the first equals sign. At line 140, every occurrence of `include-system-site-packages` assigns its boolean value, so a later entry replaces the earlier value.
- `crates/uv/src/lib.rs:1403` and `crates/uv-virtualenv/src/lib.rs:71`: absent an explicit destination, the CLI defaults the prompt to `.`; `Prompt::from_args` maps that to the current directory's name. With an explicit destination and no prompt option, `Prompt::None` omits the configuration field. The activation script can still derive a display prompt separately.
- `crates/uv/tests/python/venv.rs:1836`: `verify_pyvenv_cfg_relocatable` already supplies a prompt containing a newline and verifies Xonsh escaping and relocatable activators. It does not check injected configuration directives or Python isolation. This coverage, associated with astral-sh/uv#19740, must be considered when choosing newline handling; it is not an existing regression test for this report.
- The reporter-linked upstream commit `b0008a58885ded9227faf8f910c6c402712c750a` belongs to pypa/virtualenv#3426. Its actual changes reject line boundaries in destinations for the stdlib venv creator and stop merging the intermediate stdlib configuration. uv's inspected writer directly constructs its configuration and does not perform that merge. The analogous format problem is established; identical implementation or CVE applicability is not.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separately investigated explicit --prompt newlines, directory-derived prompts, and unintended system/user-site visibility. Literal searches included pyvenv.cfg, include-system-site-packages, ENABLE_USER_SITE, newline, and CVE-2026-113715; conceptual searches included injection, escaping, sanitization, special characters, line breaks/boundaries, readback, and venv isolation, without Fedora or version restrictions. Fix-oriented checks included closed issues, merged-PR searches, configuration-writer commit history, and linked discussions and diffs. PR keyword searches returned no hits, so referenced PRs and file history were inspected directly. Ruled out astral-sh/uv#15627 and astral-sh/uv#12739 as duplicate evidence because their discussions concern external pip or unconfirmed isolation claims; astral-sh/uv#13456 concerns prompt formatting; astral-sh/uv#20445 validates UTF-8 paths, not line boundaries. Followed astral-sh/uv#9424 through astral-sh/uv#8947, astral-sh/uv#8984, and the closed, unmerged astral-sh/uv#9466. Also inspected prompt history in astral-sh/uv#1445 and astral-sh/uv#1570, newline-containing activation coverage in astral-sh/uv#19740, and the reporter-linked upstream commit and pypa/virtualenv#3426. No existing uv tracker or previous fix for the same configuration-writing defect was found.

Before searching, the report was separated into these claims and identifiers:

- Explicit `--prompt` text can contain LF characters and introduce additional configuration directives.
- A directory name may supply the prompt text; the relevant current-directory and explicit-destination paths were checked separately.
- The observable failure is unintended system/user-site visibility without `--system-site-packages`, not merely a shell prompt display problem.
- The affected components are `uv venv`, `pyvenv.cfg` generation, and configuration consumers. The upstream lead concerns a separate readback-and-merge path.
- Exact identifiers were `pyvenv.cfg`, `--prompt`, `include-system-site-packages`, `site.ENABLE_USER_SITE`, `sys.path`, and CVE-2026-113715. No error message was reported. The reported version and platform were uv 0.13.6 and Fedora Linux.

The strongest exclusions have specific differences:

- astral-sh/uv#15627: the maintainer explains that plain `pip list` invokes pip outside an unseeded virtual environment. Its configuration says system packages are disabled; there is no newline trigger.
- astral-sh/uv#12739: the initial isolation claim is unconfirmed, and a later Fedora comment describes external pip. There is no evidence of injected configuration.
- astral-sh/uv#13456: parentheses, trailing spaces, and default prompt naming in `VIRTUAL_ENV_PROMPT`; no configuration injection.
- astral-sh/uv#20445: early validation for non-UTF-8 destination paths, merged on 2026-07-15. Valid UTF-8 newlines are outside that check.
- astral-sh/uv#9466: a batch-activator quoting proposal referenced by the older CVE discussion. Its actual status is closed and unmerged, despite an earlier comment describing it as a fix. It does not address `pyvenv.cfg`.
- astral-sh/uv#1445 and astral-sh/uv#1570 explain the historical prompt feature and defaults; neither tracks the newline defect. astral-sh/uv#15922 concerns shell/theme activation configuration.

## Suggested next step and validation limits

Prevent line boundaries in serialized configuration values from becoming independent directives. Decide the supported treatment of explicit prompt text separately from path values, which cannot be silently rewritten without changing their meaning. Validate inputs before creating or replacing the environment where rejection is chosen.

Add focused integration coverage in the existing venv test module for an explicit prompt containing actual newline characters and a current-directory-derived prompt. Check the generated configuration and the resulting Python isolation behavior; keep independent scenarios in separate named tests. Review the existing newline-containing relocatable test before changing accepted inputs, follow nearby snapshot conventions, and apply any required platform or test-feature gates.

No runtime reproduction, builds, or test execution was performed. No repository files or GitHub state were modified; this README is the maintainer handoff.
