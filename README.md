# Follow UAPI.6 Configuration Files Specification on Linux

Issue: astral-sh/uv#23145

Classification: enhancement

## Summary

The reporter requests Linux system-level configuration following the UAPI.6 Configuration Files Specification. The proposal separates packager/vendor defaults under `/usr` from administrator configuration under `/etc`, supports drop-ins that extend packaged settings, and retains the existing XDG user-level configuration behavior.

The reported example is Fedora packaging configuration at `/etc/uv/uv.toml`: an administrator should be able to customize system defaults without editing the packaged file or relying on each user's configuration. The Fedora packaging detail is supplied by the reporter; the uv configuration-discovery behavior below is independently confirmed in this checkout.

The closest open requests are astral-sh/uv#22905 for user-level configuration fragments and astral-sh/uv#5596 for explicit configuration inheritance. Closed astral-sh/uv#6742 and merged astral-sh/uv#7851 explain the current single-system-file design. None covers the complete UAPI.6 system-level proposal.

## Draft response

uv currently loads the first system configuration file found through XDG_CONFIG_DIRS (defaulting to /etc/xdg), falling back to /etc/uv/uv.toml. It merges that file with user and project configuration, but does not discover vendor defaults under /usr or load drop-ins. This follows the design in astral-sh/uv#7851.

Your proposal adds a distinct system-level capability. astral-sh/uv#22905 discusses user-level configuration fragments, and astral-sh/uv#5596 discusses explicit inheritance; both are related but have different scopes. The next step is to define how UAPI.6 system paths and drop-in precedence interact with XDG_CONFIG_DIRS and uv's existing scalar and array merging rules, while retaining XDG user configuration. This needs design agreement before implementation.

## Classification

Requests new Linux system-configuration discovery and composition capabilities: vendor defaults under /usr, administrator overrides under /etc, and drop-ins while retaining XDG user configuration. Current documentation and source consistently select one system file, and astral-sh/uv#7851 records that design choice; no incorrect implementation or returned fix is established. The related fragment and inheritance requests have materially different scopes, so they do not centralize this complete request.

The classification follows the requested capability, independently of the issue's existing enhancement label. The report contains no failing command, exact error, affected uv version, or claim that previously supported UAPI.6 behavior regressed. A reproduction is not needed to establish this feature request: source inspection confirms that the requested discovery and drop-in support are absent.

astral-sh/uv#22905 overlaps the drop-in mechanism but is explicitly about user-level files; this issue explicitly limits its proposed change to system configuration. astral-sh/uv#5596 concerns an explicit reference to another configuration file, rather than automatic discovery of vendor and administrator configuration. Neither is a duplicate of the full request.

## Related

- astral-sh/uv#22905 — Allow user-level `uv.toml` to be split (issue, open). Requests uv.toml.d fragments applied after the main file in lexicographical order. This closely overlaps the drop-in capability, but concerns user configuration across machines; astral-sh/uv#23145 explicitly retains user-level behavior and requests system-level vendor/administrator separation.
- astral-sh/uv#6742 — Provide system-level configuration files (issue, closed). Established the system-wide configuration layer for administrators, merged below user and project settings. Closed after astral-sh/uv#7851 landed; it requested a single system file, not separate vendor defaults, administrator overrides, or drop-ins.
- astral-sh/uv#7851 — Add support for system-level `uv.toml` configuration (pull request, merged). Implemented astral-sh/uv#6742 and merged on 2024-10-21. Maintainers explicitly chose the first matching XDG_CONFIG_DIRS file, defaulting to /etc/xdg, with /etc/uv/uv.toml last, rather than combining system files. This establishes the current baseline, not an implementation of UAPI.6 or evidence of a regression.
- astral-sh/uv#5596 — Feature Request: `extend` config option (issue, open). Requests explicit inheritance of shared configuration across repositories, similar to Ruff's extend directive. Related to composing configuration, but does not request automatic system drop-in discovery or vendor/administrator paths. It is labeled wish, and a maintainer said in October 2024 that it was unlikely to receive near-term work.

## Supporting evidence

- `crates/uv-dirs/src/lib.rs:130`: `locate_system_config_xdg` checks the colon-separated `XDG_CONFIG_DIRS` entries, defaults to `/etc/xdg` when unset or empty, and returns immediately on the first `uv/uv.toml` file.
- `crates/uv-dirs/src/lib.rs:163`: the Unix branch of `system_config_file` uses that XDG result first, then falls back to `/etc/uv/uv.toml`. It does not add vendor directories under `/usr` or scan drop-in directories.
- `crates/uv-settings/src/lib.rs:87`: `FilesystemOptions::system` obtains one system path, reads and validates that file, and marks its options as system-origin configuration.
- `docs/concepts/configuration-files.md:55`: the documented precedence is project over user over system. Multiple system files are not merged: the first-discovered system file is used, with XDG taking priority. Arrays across configuration levels are concatenated; scalar values use the higher-priority level.
- In astral-sh/uv#7851, maintainer discussion on 2024-10-12 specifies XDG lookup before the `/etc/uv/uv.toml` fallback. On 2024-10-16, maintainers explicitly discuss combining system files and choose the simpler first-file approach. Relevant comments: https://github.com/astral-sh/uv/pull/7851#issuecomment-2408700995 and https://github.com/astral-sh/uv/pull/7851#issuecomment-2418053281.
- astral-sh/uv#7851 merged on 2024-10-21, and astral-sh/uv#6742 closed on 2024-10-22 after that merge. This is historical system-configuration support, not a prior UAPI.6 fix.
- astral-sh/uv#22905 specifies a user-level `uv.toml.d` directory, lexicographical application after the main file, and filtering to `.toml` files. It has no comments establishing a system-level design.
- astral-sh/uv#5596 is labeled `wish` and `area:configuration`. A maintainer's 2024-10-22 comment says near-term work was unlikely; that historical statement does not establish a present commitment or rejection of this separate proposal.

## Search coverage and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Separate literal searches covered UAPI, hermetic-usr, /usr with uv.toml, /etc/uv/uv.toml, XDG_CONFIG_DIRS, drop-in/drop-ins, and uv.toml.d. Conceptual searches covered vendor/packager defaults, administrator configuration, system/global configuration, fragments, splitting, merging, includes, and inheritance, including area:configuration. Fix-oriented PR searches covered system configuration, XDG, drop-ins, and extend; followed astral-sh/uv#6742 to astral-sh/uv#7851 and read comments and reviews. Compared the candidates with current discovery/loading code and documentation. Inspected but ruled out astral-sh/uv#9985, which concerns XDG storage locations and Python executable links, and astral-sh/uv#17509, which concerns distro test isolation. The reporter's uapi-group/specifications#76 lead discusses general adoption, not an existing uv implementation. No existing item found tracks the complete UAPI.6 system-level request.

Also inspected astral-sh/uv#3420, which requested merging local and user configuration and was closed in May 2024. It does not cover system vendor/administrator precedence or drop-in discovery.

The external adoption discussion in uapi-group/specifications#76 provides motivation for hermetic system configuration. It does not establish uv support or a canonical uv discussion. No conclusion here depends on the reporter's Podman comparison.

## Next step

Agree on the system discovery and precedence design before implementation: which UAPI.6 paths to support, interaction with `XDG_CONFIG_DIRS` and the existing `/etc/uv/uv.toml` fallback, drop-in ordering and replacement semantics, and how the resulting configuration composes with uv's scalar and array merging rules. Retaining XDG user configuration is an explicit constraint of the report. Keep shared fragment semantics aligned with astral-sh/uv#22905 where appropriate; there is no implementation or delivery commitment.

Only this temporary handoff README was updated. No checkout files or GitHub objects were changed, and no builds or tests were needed for this source-and-discussion triage.
