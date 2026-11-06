# Shell script wrapping uv run?

Issue: astral-sh/uv#23105

Classification: question

## Summary

The reporter wants to invoke a command provided by a local uv project from any directory without entering the project or typing the full uv invocation. Their Bash wrapper in `~/bin` runs `uv run --project /home/me/path/to/project command "$@"`, and they say it works. The issue body explicitly corrects the original unquoted `$@`; no argument-forwarding failure is reported. The goal is everyday use of commands from uv's existing editable installation in the local Git clone's `.venv`, without making a separate installation through pipx or another tool installer. The reporter intends to check out a released Git tag rather than rely on the development branch. They ask about drawbacks, alternatives, and documenting project-command wrappers alongside the existing script-shebang guide. No platform, uv version, error, or failing behavior is supplied.

astral-sh/uv#13800 addresses global access through a separate editable tool installation, which does not meet the clarified requirement to reuse the existing project `.venv`. astral-sh/uv#14984 discusses using a project's environment from elsewhere; astral-sh/uv#11553 documents script shebangs, while astral-sh/uv#7242 separately tracks installing standalone scripts.

## Maintainer guidance

Maintainer woodruffw confirms that `--project` is the correct flag for running a command in a project's context from outside its working tree, and that wrapping this invocation in a script should work. They nevertheless do not generally encourage this pattern, preferring `uv tool install -e ...` or packaging the project as an installable distribution for clearer behavior. This answers the basic flag/wrapper question without establishing that the reporter's complete setup has been tested or endorsing symlinked executables, man pages, or completion scripts. It does not commit to a documentation change.

The separate-tool-environment recommendation remains an alternative rather than satisfying the reporter's explicit preference to reuse the existing project `.venv`. No uv failure or regression is established, so the classification remains question.

## Prior draft response — superseded by clarified scope

The retained draft below assumes that a separate tool installation is acceptable and proposes centralizing discussion in astral-sh/uv#13800. Those assumptions do not match the reporter's clarified requirement to reuse the existing project environment. It should not be used as the current response.

For an installable project that exposes this command, the recommended workflow is `uv tool install --editable /home/me/path/to/project`, as discussed in astral-sh/uv#13800. That exposes its commands globally and reflects local source changes; use `uv tool update-shell` if the tool executable directory is not on PATH. It uses a separate tool environment, with dependencies managed separately from the project's environment.

Your wrapper is also a reasonable choice when you want to use the project's own environment:

```sh
#!/bin/sh
exec uv run --project /home/me/path/to/project command "$@"
```

Quote `"$@"` to preserve arguments containing spaces or wildcard characters. `exec` avoids retaining an extra shell process. `--project` retains the caller's working directory, and `uv run` checks and syncs the project environment before running the command. The wrapper depends on the checkout remaining at that path.

The shebang documentation added in astral-sh/uv#11553 covers script files; a project-command example would clarify a different workflow. Please continue the local-tool discussion in astral-sh/uv#13800.

## Classification

Question: the reporter asks for pitfalls or improvements when exposing commands from an existing uv-managed project environment for everyday use. Both the wrapper and symlink reportedly work; no incorrect uv behavior, failed command, or regression has been established. The suggestion to document this usage remains secondary to the support question.

The explicit requirement to reuse the existing `.venv` distinguishes this request from astral-sh/uv#13800. In that issue, maintainer konstin recommends `uv tool install -e .` to expose a local repository's CLI globally, and its reporter confirms that solution. That recommendation creates a separate tool environment even though it references the same source checkout, so it does not settle the present question. The earlier duplicate interpretation is therefore superseded by the clarified scope; astral-sh/uv#13800 remains a related alternative rather than the canonical duplicate target.

A project wrapper can use the existing environment, including its editable installation and commands supplied by dependencies. The issue body correctly quotes `"$@"`; shell argument forwarding is a request for confirmation rather than a reported failure. Maintainer woodruffw confirms that the `--project` wrapper should work while generally preferring tool installation or distribution packaging. There is still no project-specific reproduction or independent verification of the reporter's complete setup.

## Execution options and clarified use case

The reporter wants commands from the local Git clone's existing editable project installation to be available outside the project without per-shell activation or a separate installation. They report that testing the `uv run --project` wrapper and a symlink from their bin directory to `/home/me/path/to/project/.venv/bin/command` continues to work. Neither approach has been reproduced independently for their project; the command's entry-point type, project configuration, uv version, and platform remain unspecified.

They also propose linking man pages from `.venv/share/man` to `~/.local/man` and any shell completion scripts to `~/.local/share/bash-completion/completions`. These paths describe the reporter's intended integration, not verified file locations or a confirmed uv-managed installation mechanism. No specific package, completion script, man page, or shell-discovery configuration is supplied. Checking out a released Git tag is their stated plan for selecting the project version, not a reported failure or a requirement for uv.

A community commenter suggests activating the project's virtual environment as another execution option. This is documented in `docs/guides/projects.md:249`: manually run `uv sync`, activate the environment, then invoke its commands directly. For the reporter's Bash wrapper scenario, activation with an absolute path such as `source /home/me/path/to/project/.venv/bin/activate` allows command lookup from other directories in that shell; `deactivate` exits the environment, as documented in `docs/pip/environments.md:84`. Unlike a wrapper or installed tool, this changes the current shell's environment and requires activation in each shell where those commands are needed. It does not provide automatic per-invocation locking or syncing. The activation template in `crates/uv-virtualenv/src/activator/activate` sets `VIRTUAL_ENV` and prepends the environment's executable directory to PATH, addressing the environment-setup distinction noted for a bare symlink. This is a source-backed alternative suggested by a community member. The reporter explicitly distinguishes it from their goal of reusing the installation through globally accessible wrappers or links; it is not a reporter-accepted resolution or a maintainer decision.

The tradeoff between these approaches has supporting repository evidence:

- `docs/concepts/projects/sync.md`, under “Automatic lock and sync,” confirms that `uv run` locks and syncs the project before invoking the command. Direct execution through a symlink does not invoke uv, so maintaining the project's environment and lockfile becomes a separate operation. The reporter correctly identifies this difference.
- Direct execution is not otherwise identical to `uv run`: `crates/uv-project-commands/src/run.rs:1195` constructs a PATH including the environment's scripts directory, and line 1227 sets `VIRTUAL_ENV` when using a virtual environment. A symlink alone does not perform that environment setup. This distinction can matter for commands that invoke other project executables or inspect those variables; no such failure is reported here.
- The reporter cites astral-sh/uv#11991 as precedent for the wrapper. Inspection confirms that maintainer charliermarsh recommends `uv run --project /root/my_project /root/my_project/task.py`, and that issue's reporter confirms it solves their cron invocation. Maintainer konstin separately explains that cron may not inherit the interactive shell's PATH. That supports explicit project selection but does not establish that the present report involves cron or has a PATH failure.

The basic `--project` wrapper question has a maintainer answer: the flag is appropriate and the wrapper should work, although this is not the maintainer's generally preferred installation pattern. The potential pitfalls of the broader link-based integration, including maintenance of manually linked ancillary files, remain unverified. The reporter reports success so far; no additional failure mechanism is established.

## Related

- astral-sh/uv#13800 — issue, closed: Recommended way to work with tools locally. Related alternative for globally exposing a local repository's CLI. A maintainer recommends `uv tool install -e .`, and the reporter confirms that it solves their request. It creates a separate tool environment, whereas the present reporter explicitly wants to reuse the already-created project `.venv`; it is not a duplicate target for that clarified requirement.
- astral-sh/uv#11991 — issue, open: Improve documentation on how to run uv script in a cron. Newly cited by the reporter and inspected directly. A maintainer recommends explicit `uv run --project` for a project script invoked outside its directory; the original reporter confirms it works. Another maintainer identifies cron-specific PATH configuration as a separate concern. This supports the project-selection pattern, while differing from a shell wrapper exposing a project command for interactive everyday use.
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

The retained prior draft is superseded by the clarified requirement; no replacement public response has been drafted. The stored initial structured triage reflects the broader global-CLI interpretation and its duplicate classification; the current handoff classifies the clarified request as a question. No GitHub comments, labels, issues, or pull requests were changed. Only this temporary issue-context README was authored; the checkout was not modified.
