# Inspecting environments

## Listing installed packages

To list all the packages in the environment:

```console
$ uv pip list
```

To list the packages in a JSON format:

```console
$ uv pip list --format json
```

To list all the packages in the environment in a `requirements.txt` format:

```console
$ uv pip freeze
```

## Inspecting a package

To show information about an installed package, e.g., `numpy`:

```console
$ uv pip show numpy
```

Multiple packages can be inspected at once.

## Verifying an environment

It is possible to install packages with conflicting requirements into an environment if installed in
multiple steps.

To check for conflicts or missing dependencies in the environment:

```console
$ uv pip check
```

Use `uv pip check --output-format json` to consume these diagnostics in another tool. A completed
check writes one JSON object to stdout and exits with status `0` when the environment is compatible
or `1` when it has incompatibilities. The default `text` output is unchanged.

The report identifies the installed environment separately from the Python version and optional
platform override used for the check. Diagnostics include the normalized package name, a `kind`, and
the relevant requirement, installed version, or metadata paths. Requirements use credential-safe
URLs, and diagnostics are sorted by package and kind. The format is experimental; pass
`--preview-features json-output` to acknowledge that its schema may change without warning. The
[JSON Schema](../reference/internals/pip-check.schema.json) is generated from the report types.

Use `--output-format jsonl --preview-features jsonl` for a single-line `"type": "result"` record
containing the same report. A completed check still exits with status `0` or `1`; a setup failure,
such as a missing interpreter, retains its usual error status without emitting a report. JSONL is a
separate preview format and may change without warning.
