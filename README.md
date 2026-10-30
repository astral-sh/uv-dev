# Annotated-tag object id given as a git rev fails with "Exact Git revision does not match precise commit"

Issue: astral-sh/uv#23018

Classification: bug

## Summary

The reported behavior is reproducible with the installed uv 0.13.5 on Linux x86_64 and Python 3.12.3. In a minimal local Git repository, passing an annotated tag's full 40-character object ID to `uv tool run --no-cache --from` fails with the exact reported “Exact Git revision ... does not match precise commit ...” diagnostic. Passing the peeled commit ID or the tag name instead succeeds and imports the package. `uv tool run` is the command behind `uvx`.

The reporter used uv 0.13.6 (Homebrew build dated 2026-10-26, aarch64-apple-darwin) on macOS 26 / Darwin 27.0.0, with the annotated `v2.32.3` tag of `psf/requests`. Their Python version was not specified. That exact platform/version combination and the public-package installation were not executed during reproduction; the local fixture isolates the same tag-object-versus-commit distinction.

The rejection is deliberate in the existing integration tests, including the test previously verified in 0.13.6. Reproducing it does not establish that accepting tag-object IDs is the intended behavior. The report asks either to accept and peel them or to explain the rejection and document how to obtain a commit pin. Diagnostic clarity and documentation remain actionable concerns; changing acceptance requires a separate policy decision.

astral-sh/uv#22466 explicitly tests this rejection, building on exact-commit validation from astral-sh/uv#21224. Neither addresses the requested annotated-tag explanation; the existing related-item investigation found no duplicate tracking that correction.

## Draft response

A full SHA revision currently has to identify a commit. astral-sh/uv#21224 added that validation, and astral-sh/uv#22466 explicitly tests rejection of annotated-tag object IDs. The two IDs you show identify the tag object and its target commit, respectively.

The error should explain that distinction and suggest the commit ID. For now, use the SHA returned by `git rev-parse 'v2.32.3^{commit}'` in a clone, or reference the tag by name with `@v2.32.3`. A focused next step is an annotated-tag-specific diagnostic and a documentation example; accepting tag-object IDs would require revisiting the exact-commit policy.

## Classification

Retain the bug classification for the reported diagnostic clarity concern, with documentation as a related improvement. The observed rejection itself is intentional and already tested: full hexadecimal revisions are treated as exact commit pins. The generic mismatch error does not explain that the requested object is an annotated tag or identify a workaround.

Accepting tag-object IDs would separately change the exact-commit policy. The related PRs establish current behavior rather than track the requested diagnostic correction. The report does not identify a last known-good version, and there is no evidence of a previously fixed annotated-tag bug returning. Both related changes predate the reported release.

## Related

- astral-sh/uv#22466 — Represent full Git revisions as commit references (merged). Parses full hexadecimal revisions as GitReference::Commit and adds install_git_full_commit_rejects_tag_object, which snapshots the exact reported error for fresh and cached annotated-tag objects. Merged October 14, 2026; the test is present in 0.13.6. Establishes intentional rejection, but does not address the requested explanatory diagnostic or documentation.
- astral-sh/uv#21224 — Ensure full Git commit pins resolve to the requested commit (merged). Introduced exact-object resolution and requested-versus-resolved commit validation to prevent SHA-shaped branches from overriding commit pins. Merged August 19, 2026 and referenced by astral-sh/uv#22466. Explains the validation constraint; its branch-substitution fix does not resolve annotated-tag diagnostic clarity.

## Reproduction

**Outcome: reproducible.** A fresh, local annotated tag reproduces the exact error without relying on GitHub or third-party package source. The peeled-commit and named-tag controls both succeed.

### Environment and isolation

- Installed executable: `/opt/hostedtoolcache/uv/0.13.5/x86_64/uv`, reporting `uv 0.13.5 (x86_64-unknown-linux-gnu)`.
- Linux x86_64, kernel `7.0.0-1017-azure`; `/usr/bin/python3` reports Python 3.12.3; Git 2.56.0.
- Fixture and per-command result logs: `/tmp/uv-23018-gmh6g8ic/`. `tag-object.json`, `peeled-commit.json`, and `named-tag.json` retain command arguments, stdout, stderr, and exit codes.
- Commands ran outside the checkout with a clean subprocess environment. Temporary files, uv cache/tool/Python locations, and XDG directories were redirected under that temporary root. Configuration discovery and Python downloads were disabled; Git global/system configuration and interactive authentication were disabled. No existing credentials or user configuration were used.
- The package has static metadata, no runtime dependencies, and the same `uv_build` requirement used by nearby integration tests. No project, workspace, dependency groups, or lockfile are involved.

### Minimal fixture and commands

Create an isolated temporary repository with this `pyproject.toml` and `src/example/__init__.py` containing `__version__ = "0.1.0"`:

```toml
[project]
name = "example"
version = "0.1.0"
requires-python = ">=3.12"

[build-system]
requires = ["uv_build>=0.7,<10000"]
build-backend = "uv_build"
```

From its temporary parent directory, create the commit and annotated tag. These are the fixture's Git operations; the fixed dates make the observed object IDs repeatable with the same file contents:

```sh
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_TERMINAL_PROMPT=0
export GIT_AUTHOR_DATE=2000-01-01T00:00:00Z
export GIT_COMMITTER_DATE=2000-01-01T00:00:00Z
git -C repository init --initial-branch=main
git -C repository add .
git -C repository -c user.name=Reproduction -c user.email=reproduction@example.invalid commit -m 'Minimal package'
git -C repository -c user.name=Reproduction -c user.email=reproduction@example.invalid tag -a v1 -m 'Annotated tag'
tag_object=$(git -C repository rev-parse v1)
peeled_commit=$(git -C repository rev-parse 'v1^{commit}')
git -C repository cat-file -t "$tag_object"
git -C repository cat-file -t "$peeled_commit"
```

The object types were `tag` and `commit`, respectively. Run these explicit commands with the temporary environment described above (`repository` is the absolute path to that repository):

```sh
repository="$PWD/repository"
uv tool run --no-cache --no-config --no-python-downloads --python /usr/bin/python3 \
  --from "git+file://$repository@$tag_object" python -c 'import example; print(example.__version__)'
uv tool run --no-cache --no-config --no-python-downloads --python /usr/bin/python3 \
  --from "git+file://$repository@$peeled_commit" python -c 'import example; print(example.__version__)'
uv tool run --no-cache --no-config --no-python-downloads --python /usr/bin/python3 \
  --from "git+file://$repository@v1" python -c 'import example; print(example.__version__)'
```

### Observed results

The tag-object command exited with code **1** and stderr:

```text
   Updating file:///tmp/uv-23018-gmh6g8ic/repository (b8dc0af5eb3a24d9c52c56dc26e12e9f8edec28a)
error: Failed to resolve `--with` requirement
  cause: Git operation failed
  cause: Exact Git revision `b8dc0af5eb3a24d9c52c56dc26e12e9f8edec28a` does not match precise commit `fc661dceae525947ba6c8038920124d940938c0f` for `file:///tmp/uv-23018-gmh6g8ic/repository`
```

The peeled-commit command and the `@v1` command each exited with code **0**, built and installed `example`, and printed `0.1.0`. Both resolved to commit `fc661dceae525947ba6c8038920124d940938c0f`. The failure therefore depends on supplying the annotated tag object's full ID, despite its target being the same installable commit.

The reporter's requested outcome is either successful peeling or a diagnostic that identifies the tag object and suggests its peeled commit, plus documentation. The observed error provides neither that explanation nor the workaround. The successful controls verify the workaround for this fixture, without deciding whether tag-object pins should become accepted.

### Existing test coverage

`crates/uv/tests/pip_install/pip_install.rs::install_git_full_commit_rejects_tag_object` (line 3386, gated by `#[cfg(feature = "test-git")]`) creates a local repository with an empty commit and an annotated `v1` tag. It obtains both object IDs, invokes `pip install` with the tag object's full ID, and snapshots exit code 1 and this exact mismatch error. A second explicit invocation checks the same rejection after the object is cached. The test's full setup and assertions were read; it directly covers the reported trigger, though through `pip install` rather than the tool command. The reproduction above independently observes the tool-command behavior and successful alternatives.

The related `install_git_full_commit_ignores_same_named_branch` test in that file checks that a full commit pin cannot be redirected by a same-named branch; it provides relevant context for the strict validation. No new tests were added or existing tests executed.

## Supporting evidence

- `crates/uv-git-types/src/reference.rs:42`: `GitReference::from_rev` parses a full object ID as `GitReference::Commit`.
- `crates/uv-git/src/git.rs:145`: the Git reference resolver runs `rev-parse <object>^0` for a commit reference. The source comment explains that `^0` peels tags and verifies a commit target.
- `crates/uv-git/src/source.rs:125`: the fetched revision is passed to `GitUrl::with_precise` before checkout.
- `crates/uv-git-types/src/lib.rs:171`: `with_precise` rejects a commit reference whose ID differs from the resolved ID; the error definition at line 88 contains neither an annotated-tag explanation nor a workaround.
- `crates/uv/tests/pip_install/pip_install.rs:3386`: `install_git_full_commit_rejects_tag_object` creates an annotated tag and expects this exact error on both the first fetch and the subsequent cached attempt. It is gated by `test-git`. The test already covers the reported trigger, so a diagnostic change should update this focused coverage rather than duplicate it.
- The GitHub contents API confirms that the same test exists in the `0.13.6` tag. astral-sh/uv#22466 merged on 2026-10-14 and astral-sh/uv#21224 merged on 2026-08-19, before the reporter's October 26 build.
- The public GitHub Git-data API confirms that `psf/requests` object `61e2240f283f15780ac2d0e2cfefb0fd6fdab627` is annotated tag `v2.32.3`, pointing to a commit object with ID `0e322af87745eff34caffe4df68456ebc20d9068`. The mismatch is between different object types for the same tagged revision.
- `docs/concepts/projects/dependencies.md:281` documents named tags and revisions (“commit”), but its Git section does not explain the annotated-tag object distinction or `git rev-parse '<tag>^{commit}'`.
- The new issue has no comments. Related PR bodies, issue comments, PR discussion comments, and review summaries were inspected. Neither related PR proposes the explanatory diagnostic requested here.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered the exact error, annotated-tag/tag-object terminology, the reported object ID, rev-parse, and the existing test identifier. Conceptual searches covered peeling, commit-ish references, full/40-character SHA pins, tag/hash ambiguity, Git diagnostics, and commit-pin documentation, including area:sources, area:error-messages, and documentation labels. Fix-oriented investigation followed Git source history and PR references, inspected both related PRs and their comments/reviews, and checked the 0.13.6 test. PR text searches returned no matches even for known relevant PRs, so coverage was supplemented by the latest 1,500 PRs across all states and file commit history. Ruled out astral-sh/uv#10779 (hexadecimal tag names, not tag-object IDs), astral-sh/uv#8050 (inconsistent lockfile commits), astral-sh/uv#16303 and astral-sh/uv#9045 (missing Git failure details, not this explicit mismatch), and astral-sh/uv#8490 (basic rev syntax). No existing item was found tracking the requested annotated-tag diagnostic correction.

The report was decomposed before searching into three claims: failure when a full SHA identifies an annotated-tag object, a misleading mismatch error, and missing commit-pin guidance. Searches for observable failures and documentation were kept separate from source-mechanism searches.

Additional inspected candidates were astral-sh/uv#2614 (tag and commit URLs conflicting across dependencies), astral-sh/uv#6336 (short revisions causing stale-lock errors), astral-sh/uv#5860 (a precise-commit locking panic), astral-sh/uv#2673 (duplicate Git lookups), astral-sh/uv#298 (branch/tag name precedence), astral-sh/uv#7866 (dynamic-version metadata after tag changes), and astral-sh/uv#7919 (newline handling in commit IDs). Their triggers and failures differ from rejection of an annotated-tag object ID.

The discussion chain from astral-sh/uv#16303 to astral-sh/uv#12713 concerns hidden Git failures and subdirectory/private-repository cases, not an explicitly reported tag-to-commit mismatch. astral-sh/uv#3368 concerns introducing more precise Git reference variants, without tracking this diagnostic.

GitHub search briefly hit its search-rate limit; later searches succeeded. PR text-search results omitted known merged PRs, so the direct PR listing and commit-history follow-up are material limits on any claim of exhaustive coverage. A supplemental search in astral-sh/uv-dev yielded no relevant result.

## Recommended follow-up

Improve the diagnostic for a verified annotated-tag object and show how to obtain and pin its target commit. Do not describe every requested/resolved-ID mismatch as an annotated tag: genuine mismatched commits must remain distinguishable. Add a short Git dependency documentation example using `git rev-parse '<tag>^{commit}'`.

If accepting tag-object IDs is considered instead, explicitly evaluate it against the exact-commit validation introduced by astral-sh/uv#21224. Update the existing annotated-tag integration snapshots and retain checks for mismatched commits and SHA-shaped branch names.

## Validation and limits

The reproduction outcome is based on three executed commands with the installed uv executable: tag-object failure, peeled-commit success, and named-tag success. Source and existing snapshot inspection corroborate these observations. The exact reported uv 0.13.6/macOS environment was not available and was not tested; no claim of a regression or of behavior on every platform is made.

The existing GitHub-related findings and search scope above are retained from the earlier issue-context investigation. This reproduction used local Git data and made no GitHub changes. No build of uv or repository test suite was run. The checkout was left unchanged, including its pre-existing modified `agents/codex/config.toml` and untracked `.issue-triage-event.json`; only this handoff and temporary reproduction artifacts were authored.
