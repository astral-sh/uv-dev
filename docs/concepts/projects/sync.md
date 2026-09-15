# Locking and syncing

Locking [resolves](../resolution.md) a project's dependencies into a
[lockfile](./layout.md#the-lockfile). Syncing installs a subset of locked packages into the
[project environment](./layout.md#the-project-environment).

## Automatic lock and sync

Locking and syncing are _automatic_ in uv. Before `uv run` executes a command, uv locks and syncs
the project. This keeps the project environment current. Commands that read the lockfile, such as
`uv tree`, also update it automatically before they run.

`--locked` disables automatic locking:

```console
$ uv run --locked ...
```

If the lockfile is not current, uv raises an error instead of updating it.

`--frozen` uses the lockfile without checking whether it is current:

```console
$ uv run --frozen ...
```

`--no-sync` runs a command without checking whether the environment is current:

```console
$ uv run --no-sync ...
```

## Checking the lockfile

uv checks whether the lockfile matches the project metadata. If `pyproject.toml` gains a dependency,
uv considers the lockfile outdated. If changed version constraints exclude a locked version, uv also
considers the lockfile outdated. If the constraints still permit the locked version, the lockfile
remains current.

`uv lock --check` checks whether the lockfile is current:

```console
$ uv lock --check
```

`--check` has the same effect as `--locked` on other commands.

!!! important

    New package releases do not make an existing lockfile outdated. Upgrading dependencies requires
    an explicit lockfile update. The [upgrading locked package versions](#upgrading-locked-package-versions)
    section describes this process.

## Creating the lockfile

Although uv creates the lockfile [automatically](#automatic-lock-and-sync), `uv lock` also creates
or updates it explicitly:

```console
$ uv lock
```

## Syncing the environment

Although uv syncs the environment [automatically](#automatic-lock-and-sync), `uv sync` also syncs it
explicitly:

```console
$ uv sync
```

Manual syncing helps editors use the correct dependency versions.

### Editable installation

During a sync, uv installs the project and other workspace members as _editable_ packages. Changes
then appear in the environment without another sync.

`--no-editable` disables this behavior.

!!! note

    If the project does not define a build system, uv does not install it. The
    [build systems](./config.md#build-systems) documentation describes this behavior.

### Handling of extraneous packages

`uv sync` performs an "exact" sync by default. It removes packages that do not appear in the
lockfile.

`--inexact` retains extra packages:

```console
$ uv sync --inexact
```

`uv run` performs an "inexact" sync by default. It installs all required packages without removing
extra packages. `--exact` enables exact syncing for `uv run`:

```console
$ uv run --exact ...
```

### Syncing optional dependencies

uv reads optional dependencies from the `[project.optional-dependencies]` table. These dependencies
are also called "extras".

By default, uv does not sync extras. `--extra` includes a named extra:

```console
$ uv sync --extra foo
```

`--all-extras` enables every extra.

The [optional dependencies](./dependencies.md#optional-dependencies) documentation describes how uv
manages these dependencies.

### Syncing development dependencies

uv reads development dependencies from the `[dependency-groups]` table defined in
[PEP 735](https://peps.python.org/pep-0735/).

By default, uv syncs the `dev` group. The [default groups](./dependencies.md#default-groups)
documentation describes how this default can change.

`--no-dev` excludes the `dev` group.

`--only-dev` installs the `dev` group _without_ the project and its dependencies.

`--all-groups`, `--no-default-groups`, `--group <name>`, `--only-group <name>`, and
`--no-group <name>` include or exclude additional groups. Like `--only-dev`, `--only-group` excludes
the project. Unlike `--only-dev`, `--only-group` also excludes default groups.

Group exclusions take precedence over inclusions. For example:

```
$ uv sync --no-group foo --group foo
```

uv does not install the `foo` group.

The [development dependencies](./dependencies.md#development-dependencies) documentation describes
how uv manages these dependencies.

## Upgrading locked package versions

With an existing `uv.lock` file, `uv sync` and `uv lock` prefer previously locked package versions.
A version changes only if the project's dependency constraints exclude the locked version.

The following command upgrades all packages:

```console
$ uv lock --upgrade
```

The following command upgrades one package to its latest version. Other packages keep their locked
versions:

```console
$ uv lock --upgrade-package <package>
```

The following command upgrades one package to a specific version:

```console
$ uv lock --upgrade-package <package>==<version>
```

Every upgrade must satisfy the project's dependency constraints. For example, an upgrade cannot
exceed an upper version bound.

!!! note

    uv applies the same preference to Git dependencies. If a Git dependency references `main`, uv
    prefers the commit SHA in an existing `uv.lock` file. `--upgrade` and `--upgrade-package` update
    the locked commit.

`uv sync` and `uv run` also accept these flags. They update both the lockfile _and_ the environment.

## Exporting the lockfile

`uv export` converts `uv.lock` into formats used by other tools or workflows. Supported formats
include `requirements.txt`, `pylock.toml` (PEP 751), and CycloneDX SBOM.

```console
$ uv export --format requirements.txt
$ uv export --format pylock.toml
$ uv export --format cyclonedx1.5
```

The [export guide](./export.md) describes all export formats and their use cases.

## Partial installations

Some workflows install dependencies in stages. For example, Docker builds can use staged
installation to improve layer caching. `uv sync` supports these flags:

- `--no-install-project`: Excludes the current project.
- `--no-install-workspace`: Excludes all workspace members, including the root project.
- `--no-install-package <NO_INSTALL_PACKAGE>`: Excludes the specified package or packages.

Each option still installs the target's dependencies. For example, `--no-install-project` skips the
_project_ without skipping its dependencies.

If a required package is omitted, these flags can leave an environment without one of its
dependencies.

## Malware checks

!!! important

    On-sync malware checking is in [preview](../preview.md). Its behavior can change before the
    feature becomes stable.

During a sync, uv can perform a lightweight lockfile scan for known malware against
[OSV](https://osv.dev). OSV lists MAL advisories from the OpenSSF's
[malicious packages database](https://github.com/ossf/malicious-packages).

If a locked dependency matches a malware advisory, uv stops the sync.

Either `audit.malware-check = true` in uv settings or `UV_MALWARE_CHECK=1` in the environment
enables malware checks.

The `audit.malware-check-url` setting or the `UV_MALWARE_CHECK_URL` environment variable selects an
alternative vulnerability service.
