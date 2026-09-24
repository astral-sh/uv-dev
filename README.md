# uv sync leaves an environment with missing NCCL library unchanged

Issue: astral-sh/uv#21968

Classification: duplicate

## Summary

On a Linux HPC system, the installed metadata for `nvidia-nccl-cu13` remained present and its
`RECORD` listed `nvidia/nccl/lib/libnccl.so.2`, but that library and the containing package directory
were absent. `uv sync --frozen` accepted the environment without repairing or reporting the missing
file; `uv sync --frozen --reinstall-package nvidia-nccl-cu13` restored it. The filesystem being full
around the incident is only a suspected trigger, and the report does not establish whether the file
loss occurred during installation, through a bad cache entry, or later.

The underlying sync behavior and requested incomplete-install detection are already tracked in
astral-sh/uv#15238. In that issue, maintainers confirmed that surviving `.dist-info` metadata can
make a distribution appear installed after its package files have disappeared, explicitly left
incomplete-install detection open, and recommended the same targeted reinstall workaround.

Repository documentation establishes that `uv pip check` checks metadata and dependency
compatibility; it does not validate every path in an installed distribution's `RECORD`. No pull
request implementing general installed-file integrity detection was found.

## Draft response

Thanks for clearly separating the observed state from the suspected trigger. uv does not currently
provide a command that validates every installed file against each distribution's `RECORD`. `uv pip
check` checks metadata and dependency compatibility, so it would not detect this missing-library
state.

This is the same incomplete-install detection gap tracked in astral-sh/uv#15238: if package files
are absent while the distribution metadata remains, a later sync can treat the package as
installed. The current targeted recovery is the `--reinstall-package` command you used. Let's
centralize the behavior and any implementation work in astral-sh/uv#15238.

The full filesystem or a corrupt cache could explain how the library disappeared, but the available
evidence does not establish either cause. If you can reproduce the transition into this state,
please add the exact uv and Python versions plus verbose install and sync logs to
astral-sh/uv#15238; that would help distinguish an installation/cache failure from later file
removal.

## Classification

Duplicate of astral-sh/uv#15238. That open issue tracks the same underlying correctness problem:
uv considers a distribution installed from its surviving metadata even when the distribution's
files are absent, and an ordinary sync does not repair it. The NCCL package and suspected
full-filesystem/cache trigger provide a different, unconfirmed path into the same state rather than
a distinct requested capability.

This is not an established regression. Merged astral-sh/uv#18943 validates and heals a wheel's
`RECORD` against the unpacked wheel before cache persistence, but it does not validate the files of
an already-installed distribution. No merged fix for installed-environment file validation was
found.

## Related

- astral-sh/uv#15238 — Canonical open match. Maintainers confirmed that package files can be gone
  while `.dist-info` remains, causing later syncs to accept the package as installed. The thread
  explicitly tracks incomplete-install detection and documents the same `--reinstall-package`
  recovery.
- astral-sh/uv#19412 — Independent open reproduction of the same observable sync failure after an
  overlapping OpenCV distribution removes shared files. Unlike astral-sh/uv#21968, its trigger is
  known.
- astral-sh/uv#16468 — Closely adjacent open report where metadata remains but package files are
  missing and reinstall repairs the package. Its `RECORD` is missing, whereas astral-sh/uv#21968 has
  a readable `RECORD` listing the absent library.
- astral-sh/uv#19683 — Closed report with the same `RECORD`-lists-missing-files symptom. Its reporter
  confirmed a stale/corrupt cache and closed it after a clean-cache installation worked; that cause
  is not established in astral-sh/uv#21968.

## Supporting evidence

Literal searches covered `libnccl.so.2`, `nvidia-nccl-cu13`, `RECORD`, missing installed or package
files, `.dist-info`, `reinstall-package`, disk quota or full filesystems, and corrupt caches.
Conceptual searches covered incomplete or broken environments, environment integrity and
verification, package-file deletion, sync accepting installed metadata, and `uv pip check`.
Fix-oriented issue and pull-request searches covered incomplete-install detection, cached-wheel
validation, RECORD healing, overlapping-package uninstall behavior, and content-addressed archives
across open, closed, and merged states.

Open astral-sh/uv#16841 and closed astral-sh/uv#19562 were plausible cache-corruption leads, but
they address invalid cached archives rather than detection of missing files in an installed
environment. Merged astral-sh/uv#18943 is similarly installation/cache focused and does not repair
the reported state. Closed astral-sh/uv#19430 discussed general incomplete-install detection but
implemented only prevention for one overlapping-package trigger and was not merged.
