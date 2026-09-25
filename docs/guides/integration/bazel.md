---
title: Using uv with Bazel
description: Use uv to resolve packages and authenticate requests in Bazel.
---

# Using uv with Bazel

For information about additional Bazel workflows with uv, see the
[`rules_py` uv guide](https://github.com/aspect-build/rules_py#dependency-resolution-with-uv) or the
[`rules_python` uv guide](https://rules-python.readthedocs.io/en/latest/pypi/lock.html#uv-pip-compile-bzlmod-only).

## Authentication

Bazel 7 and later support credential helpers through the `--credential_helper` option. To let Bazel
use credentials that uv stores, first authenticate uv with the service that hosts the required
files:

```console
$ uv auth login https://packages.example.com
```

Configure Bazel to run
[`uv auth helper`](../../concepts/authentication/cli.md#using-credentials-with-external-tools) for
matching hosts:

```text title=".bazelrc"
common --credential_helper=packages.example.com=%workspace%/bazel/uv-auth-helper
common --credential_helper=files.example.com=%workspace%/bazel/uv-auth-helper
```

Replace the host patterns with the hosts that serve the index and files that Bazel downloads.

Add the wrapper script that `.bazelrc` references:

```bash title="bazel/uv-auth-helper"
#!/usr/bin/env bash
exec uv --preview-features auth-helper auth helper --protocol=bazel "$@"
```

Make the script executable:

```console
$ chmod +x bazel/uv-auth-helper
```
