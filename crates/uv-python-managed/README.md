# uv-python-managed

Download, store, and maintain uv-managed Python installations, including the download catalog,
archive fetching and extraction, installation metadata, executable links, platform registration, and
installation patching.

The bundled catalogs in `download-metadata/` use the same release/artifact NDJSON format as the
remote Python download catalog. Each record contains a Python `version` with its provider build
identifier and an `artifacts` list. The filename identifies the provider; Pyodide records describe
CPython builds for Emscripten. The build script compresses these catalogs, and runtime loading uses
the shared NDJSON parser.

`fetch-download-metadata.py` selects the downloads to bundle and writes provider-scoped catalogs. It
retains selected `python-build-standalone` artifact fields and converts the other providers'
metadata to the same record format. Artifact variant names are independent of their provider. A
recognized trailing archive flavor is separated from the known and unknown variant names.
Compatibility rules filter unsupported artifacts, while default selection prefers archive flavors
and optimizations such as `pgo` and `lto`. Unknown variant names do not add a preference. Equally
preferred artifacts within a release follow catalog order. The interpreter's `debug` and
`freethreaded` properties determine the installation key; other variant names remain in the selected
artifact metadata.

For tools that require the public JSON manifest format, `scripts/python_download_metadata.py`
exports a compatibility manifest from the bundled catalogs.
