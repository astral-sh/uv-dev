# Inspecting environments

## Listing installed packages

List all packages in the environment:

```console
$ uv pip list
```

List the packages in JSON format:

```console
$ uv pip list --format json
```

List all packages in `requirements.txt` format:

```console
$ uv pip freeze
```

## Inspecting a package

Show information about an installed package, such as `numpy`:

```console
$ uv pip show numpy
```

Specify multiple packages to inspect them in the same command.

## Verifying an environment

Installing packages in separate steps can create conflicting requirements in an environment.

Check the environment for conflicts or missing dependencies:

```console
$ uv pip check
```
