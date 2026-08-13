---
title: Exporting a lockfile
description: Exporting a lockfile to different formats
---

# Exporting a lockfile

uv can export a lockfile to formats that other tools and workflows support. The `uv export` command
supports several output formats.

The [project layout](./layout.md) and [locking and syncing](./sync.md) documentation describe how uv
creates lockfiles.

## Overview of export formats

uv supports three export formats:

- `requirements.txt`: The traditional pip-compatible
  [requirements file format](https://pip.pypa.io/en/stable/reference/requirements-file-format/).
- `pylock.toml`: The standardized Python lockfile format defined in
  [PEP 751](https://peps.python.org/pep-0751/).
- `CycloneDX`: A standard [Software Bill of Materials (SBOM)](https://cyclonedx.org/) format.

The `--format` option selects the output format:

```console
$ uv export --format requirements.txt
$ uv export --format pylock.toml
$ uv export --format cyclonedx1.5
```

!!! tip

    By default, `uv export` writes to standard output. The `--output-file` option writes any format
    to a file:

    ```console
    $ uv export --format requirements.txt --output-file requirements.txt
    $ uv export --format pylock.toml --output-file pylock.toml
    $ uv export --format cyclonedx1.5 --output-file sbom.json
    ```

## `requirements.txt` format

The `requirements.txt` format is widely supported for Python dependencies. `pip` and other Python
package managers support this format.

### Basic usage

```console
$ uv export --format requirements.txt
```

`uv pip install` and tools such as `pip` can install the generated `requirements.txt` file.

!!! note

    Using both a `uv.lock` file and a `requirements.txt` file is not recommended. The `uv.lock`
    format supports features that the `requirements.txt` format cannot represent. uv maintainers
    welcome issues that explain why a project needs to export `uv.lock`.

## `pylock.toml` format

[PEP 751](https://peps.python.org/pep-0751/) defines a TOML-based lockfile format for Python
dependencies. uv can export a project's dependency lockfile to this format.

### Basic usage

```console
$ uv export --format pylock.toml
```

## CycloneDX SBOM format

uv can export a project's dependency lockfile as a CycloneDX Software Bill of Materials (SBOM). An
SBOM lists the software components in an application. This helps with security audits, compliance,
and supply chain review.

!!! important

    CycloneDX export is in [preview](../preview.md) and may change in any future release.

### What is CycloneDX?

[CycloneDX](https://cyclonedx.org/) is a standard format for software bills of materials. Security
scanners, vulnerability databases, and software composition analysis (SCA) tools can read this
format.

### Basic usage

The following command exports a project's lockfile as a CycloneDX SBOM:

```console
$ uv export --format cyclonedx1.5
```

The command creates a JSON-encoded CycloneDX v1.5 document that contains the project and all its
dependencies.

### SBOM Structure

The SBOM follows the [CycloneDX specification](https://cyclonedx.org/specification/overview/). uv
also adds these custom properties to components:

- `uv:package:marker`: Environment markers (e.g., `python_version >= "3.8"`)
- `uv:workspace:path`: Relative path for workspace members

## Next steps

The [locking and syncing](./sync.md) documentation and
[command reference](../../reference/cli.md#uv-export) provide more information about lockfiles and
exports.

The [packaging guide](../../guides/package.md) explains how to build and publish a project to a
package index.
