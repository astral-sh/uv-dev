# Installers PATH shim named `env` shadows coreutils env, silently swallowing every `env` invocation (exit 0, command never runs)

Issue: astral-sh/uv#22543

Classification: duplicate

## Summary

Duplicate of astral-sh/uv#19023. Upstream fix axodotdev/cargo-dist#2423 is merged, but uv 0.13.0 still pins the older cargo-dist 0.32.0.

The reporter uses uv 0.13.0 on Ubuntu Linux x86_64, with an install receipt identifying cargo-dist 0.32.0. They report that the standalone installer or self-update leaves a PATH setup script named `env` in the installation directory. When command lookup selects this helper, `env FOO=bar /bin/echo HELLO` produces no output and exits 0; invoking `/usr/bin/env` runs the command correctly. A SLURM wrapper using `env -u TMUX -u TMUX_TMPDIR tmux new-session -A` reportedly exhibits the same silent success.

The requested remedies are to retain safe sourcing while forwarding direct execution, rename or relocate the helper, or write PATH exports directly to shell profiles. These are alternative remedies for one underlying collision. The report's claim that the installer makes the helper executable is not established by the inspected source.

## Draft response

This is tracked in astral-sh/uv#19023, including reports of env calls silently doing nothing and returning 0. Please add your reproduction and the helper's file permissions there so we can keep the discussion together.

The cargo-dist 0.32.0 template creates this helper without setting executable permissions, so how it became executable on your Ubuntu system still needs checking. axodotdev/cargo-dist#2423 merged a fix that moves the helpers outside the binary directory and migrates shell references, but uv 0.13.0 still pins the older generator. Using /usr/bin/env explicitly bypasses the collision in the meantime.

## Classification

astral-sh/uv#19023 already tracks the same helper-name collision, including the exact silent-success failure and relocation proposals. The additional Ubuntu/uv 0.13.0 reproduction belongs there. The upstream fix is absent from the pinned cargo-dist 0.32.0 template, so there is no evidence of a previously shipped uv fix regressing. The claim that the installer itself sets executable permissions remains unconfirmed.

The canonical issue was opened April 16, 2026 against uv 0.9.28 and remains open with the bug label. Its original SLURM symptom was permission denied because the helper was non-executable. Later comments explicitly cover the new report's silent-success symptom: a June 26 fasd report describes ignored arguments and exit 0, and a September 17 MSYS report describes `env FOO=bar echo hi` doing nothing. These differences in shell and file permissions explain triggering conditions without creating a separate problem.

## Related

- astral-sh/uv#19023 — .local/bin/env created by installer may conflict with /usr/bin/env (issue, open). Tracks the same installer-generated env helper shadowing /usr/bin/env. The original SLURM report encountered permission denied, but later comments describe executable helpers silently ignoring commands and returning 0, including fasd on macOS and environment-isolation commands under MSYS. Maintainers discuss relocating the helper and confirm it is normally created without executable permissions.
- axodotdev/cargo-dist#2423 — Install env scripts in receipts dir instead of bin dir (pull request, merged). Upstream fix merged September 11, 2026: flat installations place env.sh and env.fish in the receipt directory, migrate shell references, and remove matching legacy helpers from the binary directory. Both uv 0.13.0 and this checkout pin cargo-dist 0.32.0, released May 22, whose template still uses the old layout; the upstream merge does not establish a shipped uv fix.

## Supporting evidence

- In astral-sh/uv#19023, maintainer woodruffw confirmed on April 16 that the helpers were non-executable and suggested moving them to a separate uv-managed directory. On June 26, the maintainer reiterated that uv's installer does not mark the helper executable. The thread includes reports of executable bits added by user tooling or development environments, plus MSYS execution without Unix execute bits. None proves how the new reporter's Ubuntu file acquired its permissions.
- The canonical issue's timeline leads to atuinsh/atuin#2873. Its July 21 maintainer comment identifies cargo-dist as the shared generator and points to axodotdev/cargo-dist#2423. The upstream PR's body, comments, review, merged state, and installer patch were inspected. Its migration checks that old helpers match generated contents before removing them and updating profile references.
- The uv 0.13.0 tag's `dist-workspace.toml:9` and this checkout both pin cargo-dist 0.32.0. The release tag configures the standard flat executable-directory layout at line 70. Source: https://raw.githubusercontent.com/astral-sh/uv/0.13.0/dist-workspace.toml
- cargo-dist v0.32.0's `cargo-dist/templates/installer/installer.sh.j2` constructs a helper path ending in `/env`. Lines 859–869 create the helper only when absent; lines 938–955 write its PATH-only shell body using a heredoc, without setting executable permissions or executing arguments. Consequently, an existing helper is also left untouched. Source: https://raw.githubusercontent.com/axodotdev/cargo-dist/v0.32.0/cargo-dist/templates/installer/installer.sh.j2
- cargo-dist 0.32.0 was published May 22, 2026. The upstream relocation PR merged September 11. uv 0.13.0 was published October 9 but still pins 0.32.0, so release dates alone must not be interpreted as uv having adopted the upstream fix.
- `docs/reference/installer.md:28` documents shell-profile modification and the persistence of `UV_NO_MODIFY_PATH` across self-updates. `crates/uv/src/commands/self_update.rs:354` reads the receipt's `modify_path` value, and the installer invocation at line 512 passes the install prefix and PATH-modification setting through.

## Search coverage and ruled-out candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered env shadowing, coreutils, silent execution, exit 0, srun, write_env_script_sh, add_install_dir_to_path, and modify_path. Conceptual and fix searches covered non-executable helpers, chmod, renaming, sourcing, libexec, shell installers, cargo-dist, and version-specific terms. Inspected candidate comments and followed astral-sh/uv#19023 through atuinsh/atuin#2873 to axodotdev/cargo-dist#2423. Ruled out astral-sh/uv#21837 and axodotdev/cargo-dist#2513, which concern false shadowing warnings from path normalization, and astral-sh/uv#14674, which concerns intentional PATH precedence. Some broad API searches hit rate limits; subsequent targeted CLI searches completed. No matching uv PR was found.

The search was decomposed before querying into observable command loss and false success; installer/self-update helper placement and shell sourcing; executable-permission and PATH-resolution conditions; and remedies that retain sourcing without colliding with the system command. Both literal identifiers and broader installer vocabulary were searched, with explicit issue/PR state searches and version-specific fix checks.

astral-sh/uv#21837 and its proposed upstream fix axodotdev/cargo-dist#2513 normalize installation paths and correct false warnings about `uv` and `uvx`; they do not address dispatching `env` to the wrong file. astral-sh/uv#14674 debates prepending a user-owned directory to PATH; it does not track the source-only helper's command-name collision.

## Verification limits and maintainer next step

The report, candidate discussions, release metadata, tagged generator source, and upstream patch were inspected without executing an installer or running the reporter's commands. Direct reads of Astral-hosted installer artifacts returned HTTP 403, so source conclusions rely on the versioned generator and uv's verified generator pin. The Ubuntu executable-permission claim remains an investigation point, not a confirmed installer action.

Centralize discussion in astral-sh/uv#19023 and request the actual helper path and file permissions there. For implementation follow-up, assess adopting a cargo-dist version containing axodotdev/cargo-dist#2423 and verify migration for existing installations. No particular uv release containing that change was identified.
