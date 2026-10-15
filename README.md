# Recommended Docker `.venv` workflow isolates app from user-installed system packages at runtime

Issue: astral-sh/uv#22586

Classification: duplicate

## Summary

The reporter builds a project with `uv sync --no-editable`, copies `/app/.venv` into a final Python Alpine image, and launches the app through its venv entry point. Users then run system `pip install pillow`; the application cannot import `PIL` from the separate system environment. The report requests an explicit explanation of this isolation and primary Docker examples that install into system Python.

Duplicate of the Docker system-install documentation request in astral-sh/uv#13621. The runtime-extension concern also appears in astral-sh/uv#13129 and closed astral-sh/uv#18774; astral-sh/uv#8085 and merged astral-sh/uv#6834 establish the supported UV_PROJECT_ENVIRONMENT alternative.

The report supplies an illustrative Dockerfile and optional-import snippet, but no uv version, execution logs, or independently verified reproduction. It does not allege a regression between uv releases.

## Draft response

The copied `.venv` excludes system-installed packages by default, so the behavior you describe is expected virtual-environment isolation. Please add this runtime optional-package example to astral-sh/uv#13621, which already tracks a complete system-install example for the Docker guide. The same runtime-extension concern is also discussed in astral-sh/uv#13129.

For your image, you can install optional packages into the app's environment explicitly: `uv pip install --python /app/.venv/bin/python pillow`, if uv is available at runtime. For system installation, the guide already documents setting `UV_PROJECT_ENVIRONMENT` to the system Python prefix before `uv sync` (typically `/usr/local` in official Python images). The final stage must then receive the system-installed files instead of `.venv`. If you sync again after adding optional packages, use `--inexact` to retain packages outside the project's dependency set.

## Classification

astral-sh/uv#13621 already tracks clearer Docker documentation and a complete system-install example, while astral-sh/uv#13129 explicitly records the same runtime-package isolation concern. The new runtime example and preference for changing the primary examples can be discussed in those existing threads. Source confirms that default project environments intentionally exclude system site-packages; no incorrect import behavior or regression of a previous fix is established.

The documentation request is the best canonical destination. astral-sh/uv#13621 asks for a complete system-install example in the same guide, while this report provides a concrete runtime-extension motivation and proposes promoting that approach to the primary examples. No maintainer commitment to changing the default examples was found. The system-install feature proposal in astral-sh/uv#13129 is a related discussion with a different implementation request.

## Related

- astral-sh/uv#13621 — Issue, open: Doc improvement: add section to Docker integration that demonstrates system level install. Closest documentation match: requests a complete Docker example using UV_PROJECT_ENVIRONMENT to install a project into system Python because the existing tip is insufficient. This report adds the runtime optional-package motivation and asks to favor system installation in the primary examples; that guidance discussion can be centralized here.
- astral-sh/uv#13129 — Issue, open: Docker container: Install packages into system environment with `uv pip install --root` or `uv sync --root`. Comments explicitly describe the same symptom: packages installed by container users in an entrypoint are unavailable to the application in its venv. This issue proposes a staging-root feature for multi-stage system installs, whereas the new report primarily requests documentation and changed examples. It remains open with the wish label; maintainers have questioned supporting --root.
- astral-sh/uv#18774 — Pull request, closed: Add `--root` flag for `uv pip install`. Its discussion explicitly cites runtime-installed plugins landing in system Python while build-time dependencies live in an isolated venv. The proposed staging-root implementation was closed without merging on April 2, 2026, with maintainers directing discussion to astral-sh/uv#13129. It predates this report and does not provide an available fix.
- astral-sh/uv#8085 — Issue, open: uv sync: support `--system` to not install dependencies into venv. Broader system-install discussion covering Docker and CI. Maintainers confirm that project commands do not support --system and recommend UV_PROJECT_ENVIRONMENT pointing to the Python installation prefix. It supports the workaround, but does not specifically track the Docker guide's runtime-extension explanation.
- astral-sh/uv#6834 — Pull request, merged: Allow customizing the project environment path with `UV_PROJECT_ENVIRONMENT`. Merged September 3, 2024; implemented the supported system-environment targeting mechanism for project commands. A maintainer demonstrated installation into /usr/local in Docker and warned that exact synchronization removes unrelated packages. This establishes existing functionality, not a fix for default venv isolation or evidence of a regression.

## Supporting evidence

- `docs/guides/integration/docker.md:202` describes activating the project environment or using `uv run`. The tip at line 215 already points to `UV_PROJECT_ENVIRONMENT` for installing into system Python. The guide therefore already documents the alternative, although this section does not explain the runtime optional-package scenario.
- `docs/guides/integration/docker.md:468` explains copying a non-editable project environment between build stages; its example runs the application from `/app/.venv/bin`. The pip-interface section at line 519 separately recommends `uv pip install --system` as appropriate in containers.
- `crates/uv-environment-operations/src/lib.rs:1056` creates project environments with system-site-package access set to false and `Seed::Disabled`. `crates/uv-virtualenv/src/virtualenv.rs:564` writes the corresponding `include-system-site-packages = false` setting. These source paths establish intentional default isolation and explain why adding the venv to PATH alone does not supply a venv-local pip.
- `docs/pip/environments.md:100` documents explicit interpreter targeting with `uv pip install --python /path/to/python`. This supports installing runtime extras into the same environment as the app. The reporter's final image does not copy uv, so the draft makes this command conditional on uv being available at runtime.
- `docs/concepts/projects/config.md:162` documents `UV_PROJECT_ENVIRONMENT`, the Python installation prefix, and use in a single-project Docker or CI environment. System installation changes the artifact paths: merely setting the variable while retaining `COPY /app/.venv` would not produce the intended final image.
- `docs/concepts/projects/sync.md:93` documents exact synchronization and `--inexact` for retaining additional packages. This matters if synchronization is repeated after users install optional packages.
- `crates/uv-cli/src/lib.rs:3001` documents `uv venv --system-site-packages` as an opt-in for runtime access to system packages, while explicitly distinguishing it from uv's package listing and installation behavior. The report does not use this opt-in.
- Maintainers in astral-sh/uv#8085 recommend `UV_PROJECT_ENVIRONMENT` and confirm `/usr/local` as an example installation prefix. The implementation and Docker demonstration in astral-sh/uv#6834 predate the report by more than two years.
- The comments in astral-sh/uv#13129 and astral-sh/uv#18774 explicitly mention user-installed runtime packages or plugins being outside the build-time venv. Their `--root` proposal was not accepted; it should not be presented as an existing solution.

## Searches and exclusions

The report was decomposed before searching into three claims:

1. Runtime optional imports fail after a user installs packages into system Python while the app runs in a copied venv. Observable identifiers were `pip install pillow`, `PIL`, `ImportError`, `/app/.venv`, and a multi-stage Docker build; no exact traceback was supplied.
2. The Docker guide should explicitly explain the resulting separation between system and project packages.
3. The primary Docker examples should favor system installation, with container isolation as the motivation.

Searched astral-sh/uv open and closed issues separately for runtime imports after global pip installs, missing Docker guidance, and system-install examples. Literal terms included pillow, ImportError, .venv, pip install, and system packages; conceptual searches used runtime extensions, global imports, container isolation, system-level installs, UV_PROJECT_ENVIRONMENT, and system-site-packages, including the documentation label. Removed package and platform details and followed comments and references. Attempted open, closed, and merged PR searches for Docker and environment settings; these returned no PR hits despite known matching PRs, so PR coverage relied on direct inspection of linked PRs and a recent all-state listing. Ruled out astral-sh/uv#2500 and astral-sh/uv#12800 as package-discovery/resolution issues, and astral-sh/uv#11829 with merged astral-sh/uv#11873 as a distinct layered-environment inheritance bug. Also followed astral-sh/uv#2265 to astral-sh/uv#1330 for the adjacent unseeded-pip behavior.

Additional candidate comparisons:

- astral-sh/uv#2500 concerns uv package discovery across `sys.path`, including environments that already opt into system packages. astral-sh/uv#12800 was directed there by a maintainer because installed dependencies were not reused during installation. Neither establishes incorrect Python import isolation in this report.
- astral-sh/uv#11829 used an explicitly system-enabled venv and lost that setting in a layered `uv run --with` environment. Its fix, astral-sh/uv#11873, merged on March 1, 2025. This report uses neither system-site-package inheritance nor an overlay, so there is no evidence that fix regressed.
- astral-sh/uv#2265 describes bare pip targeting global Python after activating an unseeded uv venv. A maintainer confirmed `uv venv --seed` supplies pip and centralized its documentation request in astral-sh/uv#1330. This supports the installation-target distinction but does not track the Docker system-install examples.
- astral-sh/uv#12739 reports the opposite apparent symptom, global packages showing up in a fresh venv; its comments discuss invoking global pip from an unseeded venv. It is not the runtime import-isolation request here.

PR search results were incomplete: `gh search prs` and `gh pr list --search` returned empty results for broad Docker and exact environment-setting queries, while search API responses did not yield PR records. Relevant PRs were obtained from issue references and fetched directly, and the latest 100 all-state PRs were also inspected. This limits claims of exhaustive PR coverage.

## Validation and scope

This handoff is based on the checked-out documentation and source, issue bodies, maintainer comments, and directly inspected PR status and discussions. No Docker reproduction or test suite was run. The only authored file is this README outside the checkout; no GitHub changes were made.
