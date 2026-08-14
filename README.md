# Global python pin file environment variable

Issue: astral-sh/uv#20993

Classification: enhancement

## Summary

The reporter wants the global Python pin file to be relocatable, for example with
`UV_PYTHON_GLOBAL_PIN_FILE`, so uv does not need to use `%APPDATA%\uv\.python-version` on Windows.
Their functional motivation is that `UV_PYTHON` is not equivalent to a global pin during tool
upgrades: `UV_PYTHON` supplies an explicit interpreter request to `uv tool upgrade --all` and can
migrate existing tool environments, while a global pin acts as a default for new tool installs and
does not change an existing tool during an ordinary upgrade.

No duplicate was found. The original global-pin feature deliberately stores `.python-version` in
the user-level uv configuration directory, and the later tool-operation work deliberately preserved
the distinction between a fallback global pin and an explicit Python request. An open adjacent
request proposes a system-wide, lower-precedence pin, but it does not allow a user-selected file
path.

## Draft response

Thanks, the tool-upgrade example clarifies why `UV_PYTHON` is not equivalent here. Today,
`uv python pin --global` reads and writes `.python-version` in uv's user configuration directory,
and there is no override for that file's location. `UV_PYTHON` is wired as an explicit Python
request for `uv tool upgrade`, like `--python`, so it can move existing tool environments to that
interpreter. The global pin behavior was intentionally designed as a fallback for new installs
without changing an existing tool on an ordinary upgrade in astral-sh/uv#12921 and
astral-sh/uv#14112.

A configurable global-pin path would therefore be a new capability, not something currently
covered by those discussions. The main design question is whether the override should name this
specific file or relocate the user-level uv configuration directory more generally, along with its
precedence relative to `--global` and `--no-config`. We can keep this issue open as the enhancement
request for that decision.

## Classification

This is an enhancement. The requested environment variable or equivalent path override does not
exist: the current source constructs both global-pin discovery and writes from the uv user
configuration directory, and the documentation describes that location. The report asks for a new
configuration surface rather than identifying a violation of documented behavior.

The tool-upgrade consequence supports the request but does not turn it into a bug. The CLI maps
`UV_PYTHON` to the `--python` option for `uv tool upgrade`, and the integration tests confirm that
an explicit Python request upgrades tool environments to that Python. In contrast, the accepted
design in astral-sh/uv#12921 and merged implementation in astral-sh/uv#14112 make a global pin a
fallback for new tool installations without changing an existing tool on an ordinary upgrade.

This is not a duplicate because no inspected issue or pull request requests a user-configurable
global-pin file path. astral-sh/uv#13577 is adjacent, but its system-level default has different
scope and precedence.

## Related issues and pull requests

- astral-sh/uv#13577, open issue, “System global `.python-version` `uv python pin --system`”. This
  is the closest open request for another global-pin storage location. It proposes a system-wide
  default for administrators, below the existing user global pin in precedence, rather than an
  arbitrary user-selected global-pin file.
- astral-sh/uv#4972, closed issue, “`uv python pin` should support \"global\" pins”. This is the
  original global-pin design request and explicitly describes storing the user-level default in
  the uv configuration directory.
- astral-sh/uv#12115, merged pull request, “Add support for global `uv python pin`”. This closed
  astral-sh/uv#4972, introduced `uv python pin --global`, and implemented the file under the
  user-level uv configuration directory without a path override.
- astral-sh/uv#12921, closed issue, “Respect `--global` Python pins during `uv tool` operations”.
  This is the canonical design discussion for how global pins should affect new installs,
  reinstalls, and upgrades, which is the behavioral distinction motivating astral-sh/uv#20993.
- astral-sh/uv#14112, merged pull request, “Respect global Python version pins in `uv tool run` and
  `uv tool install`”. This implemented the relevant behavior: a global pin selects Python for a
  new tool install but does not change an existing tool during an ordinary upgrade; explicit
  Python requests remain overrides.

## Search and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal terms
included `UV_PYTHON_GLOBAL_PIN_FILE`, `uv python pin --global`, `global pin file`,
`.python-version`, `%APPDATA%`, `UV_CONFIG_FILE`, and `UV_CONFIG_DIR`. Conceptual searches covered
custom, user, and system configuration directories; alternate global-pin locations; default Python
selection; `UV_PYTHON` and `--python` precedence; tool interpreter migration;
`uv tool upgrade --all`; and freethreaded upgrades. Fix-oriented inspection followed the original
global-pin issue to astral-sh/uv#12115 and the tool-semantics issue to astral-sh/uv#14112, including
their discussion and current integration tests.

astral-sh/uv#11534 was inspected because it also reports `uv tool upgrade --all` failures and
subsequent reinstall work. It was ruled out because its trigger is deletion of the Python
installation backing existing tools, not an explicit interpreter request or the global-pin file
location. astral-sh/uv#13402 surfaced for `UV_CONFIG_DIR` terminology but concerns pyenv shim
failures caused by local `.python-version` files, so it is not meaningfully related.

Current repository evidence:

- `PythonVersionFile::find_global` and `PythonVersionFile::global` both derive the path from
  `user_uv_config_dir()` and append `.python-version`.
- On Windows, the uv user configuration directory is derived from the platform configuration
  directory and then suffixed with `uv`; there is no global-pin-specific environment override in
  the environment-variable registry.
- `ToolUpgradeArgs::python` uses `UV_PYTHON` as the environment source for the same option exposed
  as `--python`/`-p`.
- The tool integration tests demonstrate that an explicit Python request moves tool environments,
  while the global-pin tests and astral-sh/uv#14112 preserve an existing tool's interpreter on a
  normal upgrade and apply a changed global pin on reinstall when no explicit request was recorded.
