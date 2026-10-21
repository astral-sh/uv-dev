# `uv run --no-project` should  ignore workspace-root `.python-version` files

Issue: astral-sh/uv#22768

Classification: enhancement

## Summary

The reporter says `uv run --no-project` still uses the workspace-root
`.python-version`, overriding the global pin created by `uv python pin --global`.
They request skipping that workspace-root file and falling back to the user configuration
pin (shown as `~/.config/uv/.python-version`), while keeping normal `uv run` behavior
and explicit `--python` precedence.

No duplicate found. astral-sh/uv#6361 and astral-sh/uv#12115 establish the current local-first behavior. Related discussions and fixes cover environment isolation, script requirement conflicts, and global pins for tool commands.

The issue was opened on 2026-10-20. It provides no uv version, platform, command output,
directory fixture, or indication that a previously working release changed behavior.
The reported selection order is supported by the inspected source; a runtime reproduction
was not performed.

Maintainer zanieb questions coupling version-file discovery to `--no-project`: in the
2026-10-21 comment on astral-sh/uv#22768 (comment ID 6307777480), they point out that
`.python-version` files can appear anywhere in the directory hierarchy and are not
inherently tied to a project root. This adds a design objection to treating a pin as
project-owned based on its location. It is consistent with the discovery implementation
described below, but does not constitute a final acceptance or rejection of the request.
The enhancement classification remains appropriate; no new reproduction details were supplied.

## Requested alternatives and design scope

The reporter's follow-up on astral-sh/uv#22768 (comment ID 6308108347) broadens the
request to include explicit selection of the global pin, rather than requiring a change
to `--no-project`. They propose these alternatives:

- Have `uv run --no-project` skip only the `.python-version` in the current working
  directory. This differs from the original workspace-root scope when invoked from a
  member or another subdirectory.
- Add a dedicated `--no-local-python-pin` or `--global-python-pin` flag to bypass local
  pins or select the global pin explicitly.
- Extend `--python` or `--python-preference` with a way to select the global pin.

These are reporter proposals, not accepted designs or available workarounds. Maintainer
zanieb asks the reporter to explain their use case in astral-sh/uv#22768 (comment ID
6308123313). The supplied discussion does not yet include that explanation or a decision
on the alternatives, and adds no affected version, platform, or reproduction fixture.

Skipping only the current directory would not by itself guarantee use of the global pin:
`PythonVersionFile::discover` searches ancestor directories before falling back to the
user configuration directory (`crates/uv-python-discovery/src/version_files.rs:109`).
An ancestor pin could therefore still take precedence if the remaining discovery order
were retained. A design needs to distinguish current-directory suppression from bypassing
all local/ancestor pins or explicitly selecting the global pin.

## Draft response

Currently, `--no-project` disables project discovery but still reads `.python-version` files, including local pins. That behavior was deliberately added for runs outside projects in astral-sh/uv#6361; the global pin is a fallback when no local pin is found.

Your request would change that selection policy. It needs a decision about whether `--no-project` should skip only workspace-root pins or all local pins while retaining the global fallback. For now, use `uv run --no-project --python <version> ...` to override the local pin explicitly. `--no-config` skips global pins too, so it does not provide the fallback you want.

## Classification

The request changes established Python-selection semantics: --no-project leaves ordinary version-file discovery enabled, local pins precede the global fallback, and explicit --python already takes precedence. The source, the intentional non-project support in astral-sh/uv#6361, and the global-pin design in astral-sh/uv#12115 agree on that behavior. No existing item tracks the same requested change, and the script-conflict fixes address different conditions rather than a demonstrated regression here.

This is an affirmative request to change selection policy, rather than a support question.
The distinction rests on intentional behavior established by implementation history and
maintainer decisions, not merely on the presence of documentation. No failure of explicit
`--python` precedence is reported or established.

## Related

- astral-sh/uv#6361 — Respect `.python-version` files in `uv run` outside projects (pull request, merged). Intentionally added local version-file discovery to the non-project run path, while preserving explicit --python precedence. It fixed astral-sh/uv#6285 and shipped in uv 0.3.1; this report requests changing that established behavior, not restoring that fix.
- astral-sh/uv#12115 — Add support for global `uv python pin` (pull request, merged). Introduced global pins in uv 0.6.7 explicitly as a fallback when no version file is found locally or in ancestor directories. Its description and maintainer review establish the local-before-global precedence involved here.
- astral-sh/uv#16578 — `--no-project` has no effect if a project virtual environment already exists (.venv) (issue, closed). Maintainer clarification says --no-project disables project discovery but retains general environment discovery; closed as intended. Its logs also show a local .python-version being read. The requested change there concerns .venv isolation, not skipping local pins in favor of a global pin.
- astral-sh/uv#9393 — Script inline python requirement not respected when .python-version file is in project (issue, open). Discusses whether a project-local pin should influence self-contained scripts. It concerns a pin conflicting with PEP 723 requires-python, whereas this report requests global-pin fallback under --no-project without reporting any script requirement conflict.
- astral-sh/uv#18097 — Fix Python version selection for scripts with a `requires-python` conflicting with `.python-version` (pull request, merged). Shipped in uv 0.10.6 and filters incompatible pins when selecting Python for scripts. This is a relevant historical fix, but it does not make --no-project ignore workspace-root pins generally; the new report does not establish a regression of this fix.
- astral-sh/uv#14112 — Respect global Python version pins in `uv tool run` and `uv tool install` (pull request, merged). Implemented global-pin selection for tool commands, closing astral-sh/uv#12921 and shipping in uv 0.10.0. Current tool discovery explicitly skips local version files, providing a related implementation precedent; uv run --no-project uses a different discovery path.

## Supporting evidence

Source was inspected at checkout commit `0713adcfb943946e5d710852e8a864c8781dee19`.

- `crates/uv-cli/src/lib.rs:3450` describes `--no-project` as avoiding project/workspace
  discovery and explicitly allows ordinary virtual-environment discovery.
- `crates/uv-project-commands/src/run.rs:571` discards the discovered project when
  `no_project` is set. The non-project branch at line 851 first accepts an explicit Python
  request; otherwise it calls `PythonVersionFile::discover` with ordinary configuration
  discovery. It does not request global-only discovery.
- `crates/uv-python-discovery/src/version_files.rs:109` searches local/ancestor version
  files first and then the global file. The `no_local` discovery option can bypass local
  files. At line 145, disabled configuration discovery returns no pin, including a
  discovered global pin; therefore `--no-config` does not implement the requested fallback.
- `docs/concepts/python-versions.md:61` documents local/ancestor files before the user
  configuration directory. The global pin was introduced with that fallback policy in
  astral-sh/uv#12115, including an explicit maintainer review comment describing it.
- `crates/uv-tool-commands/src/common.rs:209` handles explicit Python requests before
  global-file discovery and sets `with_no_local(true)`. This is an existing capability
  used by a different command family, not proof that `uv run` should use the same policy.
- `crates/uv/tests/project/run.rs:3512` (`run_no_project`) checks that project discovery
  is ignored while an existing environment may still be used.
  `run_isolated_python_version` at line 3421 checks that `--isolated` still honors a local
  pin. These tests support the distinction between project discovery, environment isolation,
  and version selection; they do not reproduce the exact competing local/global-pin case.

Release history supports longstanding behavior rather than a demonstrated regression:
astral-sh/uv#6361 shipped in 0.3.1 (`changelogs/0.3.x.md:172`), global pins in
astral-sh/uv#12115 shipped in 0.6.7 (`changelogs/0.6.x.md:423`), tool support in
astral-sh/uv#14112 shipped in 0.10.0 (`changelogs/0.10.x.md:60`), and script-conflict
handling in astral-sh/uv#18097 shipped in 0.10.6 (`changelogs/0.10.x.md:331`).

## Search coverage and exclusions

The report was decomposed before searching into three claims: skipping a workspace-root
pin under `--no-project`; retaining global-pin fallback; and preserving normal-run and
explicit-`--python` precedence. There is no exact error or version-specific regression
claim to search.

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Literal searches covered no-project, .python-version, global pins, and precedence; conceptual searches covered interpreter discovery, workspace boundaries, local pins, version files, isolation, and non-project execution. Closed-issue and fix-oriented searches covered historical pin and script-selection changes. PR keyword searches unexpectedly returned no results, so a paginated REST listing of all 13,175 PRs was filtered by relevant titles and body combinations. Inspected candidate bodies, comments, reviews, linked timelines, patches, source, tests, and changelogs. Ruled out astral-sh/uv#14916 and astral-sh/uv#15473 as incompatible-global-pin/project-requirement behavior, and astral-sh/uv#7815 as init/pin file placement. No same-scope duplicate or regression tracker was found.

Additional distinctions from inspected candidates:

- astral-sh/uv#6285 reported the opposite behavior: local pins were ignored outside projects.
  Its timeline points to the intentional fix in astral-sh/uv#6361.
- astral-sh/uv#14916 and its merged fix astral-sh/uv#15473 concern ignoring an incompatible
  global pin when a project requires another Python version. That is different from
  preferring a global pin over a valid local pin without project discovery.
- astral-sh/uv#7815 was closed without merging and changes where `uv init` and
  `uv python pin` read/write version files; it does not track `uv run --no-project`.
- astral-sh/uv#15951 concerns a global pin overriding an existing environment when there
  is no project; astral-sh/uv#20993 requests relocating the global pin file. Neither tracks
  ignoring a workspace-root pin.
- astral-sh/uv#16984 concerns prerelease interpreter selection with an explicit Python
  request, and astral-sh/uv#7562 concerns managed/system interpreter preference and pyenv
  caching. Shared discovery terminology does not make them duplicates.

## Maintainer next step

Await the reporter's use-case explanation requested by the maintainer. The concrete
workflow motivating global-pin selection is still missing; the proposed flags alone do
not establish that context. Use that information to assess whether a behavior change or
an additional selector is warranted.

Decide whether to change `--no-project` semantics or provide an explicit way to select
the global pin. A change to `--no-project` still needs to address the maintainer's
objection that version files are not inherently project-owned; the reporter also proposes
separate controls that need not change project-discovery semantics.

For any accepted approach, define whether it skips the current-directory file, the
workspace-root file, or all local/ancestor pins, and what happens when no global pin
exists. The existing `no_local` option skips all local files. Retain the requested
explicit-`--python` precedence and specify its interaction with any new selector. Behavior
should be defined from both the workspace root and a member directory.

For immediate use, the reporter can pass the desired version explicitly with
`uv run --no-project --python <version> ...`. Neither `--isolated` nor `--no-config`
alone supplies the requested local-pin suppression with global-pin fallback.

Only this handoff was written. No code, tests, or other checkout files were changed,
and no GitHub state was modified.
