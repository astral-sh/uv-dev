# Shell script wrapping uv run?

Issue: astral-sh/uv#23105

Classification: duplicate

## Summary

The reporter wants to invoke a command provided by a local uv project from any directory without entering the project or typing the full uv invocation. Their Bash wrapper in `~/bin` runs `uv run --project /home/me/path/to/project command $@`, and they say it works. They ask about drawbacks, alternatives, and documenting project-command wrappers alongside the existing script-shebang guide. No platform, uv version, error, or failing behavior is supplied.

astral-sh/uv#13800 covers the same local CLI workflow and recommends editable tool installation. astral-sh/uv#14984 discusses using a project's environment from elsewhere; astral-sh/uv#11553 documents script shebangs, while astral-sh/uv#7242 separately tracks installing standalone scripts.

## Draft response

For an installable project that exposes this command, the recommended workflow is `uv tool install --editable /home/me/path/to/project`, as discussed in astral-sh/uv#13800. That exposes its commands globally and reflects local source changes; use `uv tool update-shell` if the tool executable directory is not on PATH. It uses a separate tool environment, with dependencies managed separately from the project's environment.

Your wrapper is also a reasonable choice when you want to use the project's own environment:

```sh
#!/bin/sh
exec uv run --project /home/me/path/to/project command "$@"
```

Quote `"$@"` to preserve arguments containing spaces or wildcard characters. `exec` avoids retaining an extra shell process. `--project` retains the caller's working directory, and `uv run` checks and syncs the project environment before running the command. The wrapper depends on the checkout remaining at that path.

The shebang documentation added in astral-sh/uv#11553 covers script files; a project-command example would clarify a different workflow. Please continue the local-tool discussion in astral-sh/uv#13800.

## Classification

The central request—invoking a local project's CLI from anywhere without entering the project—is already answered in astral-sh/uv#13800 with editable tool installation. The wrapper example and documentation suggestion add detail to that workflow and can be discussed there. The reporter says the wrapper works; no incorrect uv behavior or regression is established.

The matching requirement in astral-sh/uv#13800 is global access to a CLI from a local repository. On June 3, 2025, maintainer konstin recommended `uv tool install -e .`, explaining that local source changes remain visible in the globally available command. The reporter confirmed that this met their need. That answered support discussion provides a concrete place to centralize this question.

The distinction between tool installation and a project wrapper matters: editable tool installation requires an installable package exposing the desired command and creates its own tool environment. A wrapper remains useful when the command must run with the project's environment, including commands supplied by project dependencies. The unquoted `$@` in the example is a shell argument-forwarding problem, not evidence of a uv bug. The documentation suggestion does not establish a separate broken behavior or a regression.

## Related

- astral-sh/uv#13800 — issue, closed: Recommended way to work with tools locally. Same core request: make a CLI from a local repository available everywhere while retaining local development. A maintainer recommends `uv tool install -e .`, and the reporter confirms that it solves the request. This is the canonical local-tool workflow discussion.
- astral-sh/uv#14984 — issue, open: Is this the right command to start a script from within its own dedicated project venv?. Closely related support question about using `uv run --project /script/ ...` from another project's directory while selecting the script's own environment. It concerns a Python script and active-environment behavior rather than exposing a packaged command on PATH; it has no maintainer answer.
- astral-sh/uv#11553 — pull request, merged: [docs] Add shebang section for scripts. Added the exact script-shebang documentation cited by the reporter. Its discussion includes a maintainer's shell wrapper using `exec uv run --script "$@"`. This supports the wrapper pattern, but covers running script files rather than installing a local project's command.
- astral-sh/uv#7242 — issue, open: Allow installation of scripts, e.g., `uv tool install --script <path>` or `uv script install`. Adjacent request for globally installing standalone Python scripts; maintainers support the idea but require design. The closed global-CLI question astral-sh/uv#10997 points here. Unlike this report's existing project command, it concerns scripts that are not installable tool packages, so it is not the duplicate target.

## Supporting evidence

- `crates/uv-cli/src/lib.rs:320` documents `--directory` as changing the working directory. The `--project` documentation at line 328 selects project discovery and its environment while resolving other relative arguments against the caller's working directory. The wrapper therefore does not implicitly run the command from the project directory.
- `docs/concepts/projects/run.md:7` explains that `uv run` uses the project environment and ensures it is up to date before running a command. It explicitly supports commands provided by that environment. This accounts for the environment check/sync on each wrapper invocation.
- `docs/guides/tools.md` under “Installing tools” explains persistent, isolated tool environments, executable installation on PATH, and `uv tool update-shell`. `docs/concepts/tools.md` distinguishes tool environments from the current project's environment.
- `crates/uv-cli/src/lib.rs`, in `ToolInstallArgs`, documents editable installation as reflecting source-directory changes without reinstalling. `crates/uv-tool-commands/src/install.rs:113` constructs an editable package requirement, and its installation path resolves the tool's requirements separately.
- The existing integration test `tool_install_editable` in `crates/uv/tests/tool/tool_install.rs:1693` installs a local package with `-e` and verifies installation of its executable in the tool executable directory. No new tests or reproductions are needed to classify this support request.
- `docs/concepts/projects/config.md` describes command entry points in `[project.scripts]` and the need for a build system. The editable-tool recommendation is conditional on the project being installable and actually exposing the requested command.
- astral-sh/uv#11553 added the reporter's cited documentation at https://docs.astral.sh/uv/guides/scripts/#using-a-shebang-to-create-an-executable-file. Maintainer EliteTK's discussion there includes a PATH wrapper containing `exec uv run --script "$@"`. It provides precedent for quoting and executing through a shell wrapper, while targeting Python script files.

## Search scope and distinctions

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests using the authenticated gh CLI. Literal searches covered uv run with wrapper, --project, command, anywhere, and shebang; conceptual searches covered global/local CLIs, editable tool installation, PATH, shims, and documentation. PR searches returned no hits; references in issue comments led to merged PRs astral-sh/uv#11553, astral-sh/uv#17423, and astral-sh/uv#20225, which were inspected directly. Followed astral-sh/uv#10997 to astral-sh/uv#7242 and the script-discovery discussions astral-sh/uv#12193 and astral-sh/uv#14585 to their implementation and stabilization: script-relative project discovery became default in uv 0.12.0, but this report specifies --project and reports no failure or regression. Ruled out the plausible wrapper-title match astral-sh/uv#6662 because it concerns bootstrapping/versioning uv itself; also distinguished task definitions in astral-sh/uv#5903, venv-layout symlinks in astral-sh/uv#15800, and alternate script interpreters in astral-sh/uv#18115. Checked CLI help, tools/project documentation, tool-install implementation, and existing editable-tool tests.

The report was decomposed before searching into two claims: global invocation of an existing local-project command, and documentation of that command-wrapper pattern. Searches treated those separately from possible implementation mechanisms such as editable installation, shell wrappers, and project discovery. There is no version-specific failure or exact error to search.

Additional inspected candidates were distinguished as follows:

- astral-sh/uv#12193 and astral-sh/uv#14585 concern discovering a project from a Python script's location. Maintainer comments dated October 16, 2026 state that this became the default in uv 0.12.0 through merged astral-sh/uv#17423 and astral-sh/uv#20225. These historical changes do not establish a regression here: the reporter explicitly supplies `--project`, invokes a command, and reports success.
- astral-sh/uv#10997 concerns placing a standalone PEP 723 script on PATH. A maintainer redirects it to astral-sh/uv#7242. That chain identifies the standalone-script discussion, rather than a missing ability to install an existing packaged project command.
- astral-sh/uv#6662 is about a Gradle-style wrapper that installs or selects uv itself for a project. It is a plausible title match but requests a different capability.
- astral-sh/uv#5903 requests task definitions and aliases, astral-sh/uv#15800 requests a shared cross-platform virtual-environment executable layout, and astral-sh/uv#10465 proposes dispatch based on renaming the uv binary. None is required for the reported workflow.
- astral-sh/uv#18115 requests using another tool, such as pytest, as the interpreter for a script with inline metadata. astral-sh/uv#16241 requests a dedicated executable for script shebangs. Neither asks how to expose an existing local-project CLI globally.
- astral-sh/uv#11225 discusses the resident uv parent process and memory overhead in a worker deployment. The current project-running documentation confirms that uv retains a parent process for error handling and signal forwarding; the old report's measured memory figure is not asserted for this issue.
- astral-sh/uv#12153 requests the script-shebang documentation already supplied by astral-sh/uv#11553. It does not cover a project-command wrapper example.

## Handoff status

The proposed response is a draft for maintainer review. No GitHub comments, labels, issues, or pull requests were changed. Only this temporary issue-context README was authored; the checkout was not modified.
