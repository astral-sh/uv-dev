# Package indexes

By default, uv uses the [Python Package Index (PyPI)](https://pypi.org) to resolve and install
packages. The `[[tool.uv.index]]` setting and `--index` option also support other package indexes,
including private indexes.

## Defining an index

To include another index when uv resolves dependencies, add a `[[tool.uv.index]]` entry to
`pyproject.toml`:

```toml
[[tool.uv.index]]
# Optional name for the index.
name = "pytorch"
# Required URL for the index.
url = "https://download.pytorch.org/whl/cpu"
```

uv searches indexes in their defined order. The first index in the configuration file has the
highest priority. Command-line indexes take precedence over indexes in the configuration file.

By default, PyPI is the "default" index. uv searches this index when it cannot find a package on any
other index. To exclude PyPI, set `default = true` on another index or use `--default-index`:

```toml
[[tool.uv.index]]
name = "pytorch"
url = "https://download.pytorch.org/whl/cpu"
default = true
```

The default index always has the lowest priority, regardless of its position in the list.

Index names must contain only ASCII letters, numbers, dashes, underscores, and periods.

The `--index` and `--default-index` options accept an index URL, a configured index name, or the
`<name>=<url>` syntax. The `UV_INDEX` and `UV_DEFAULT_INDEX` environment variables accept the same
values:

```shell
# On the command line.
$ uv lock --index pytorch=https://download.pytorch.org/whl/cpu
# Via an environment variable.
$ UV_INDEX=pytorch=https://download.pytorch.org/whl/cpu uv lock
```

With `--preview-features index-by-name`, configured index names take precedence over matching paths.

## Pinning a package to an index

To pin a package to an index, specify that index in its `tool.uv.sources` entry. For example, the
following `pyproject.toml` entries ensure that `torch` _always_ comes from the `pytorch` index:

```toml
[tool.uv.sources]
torch = { index = "pytorch" }

[[tool.uv.index]]
name = "pytorch"
url = "https://download.pytorch.org/whl/cpu"
```

To select different indexes for different platforms, specify a list of sources with environment
markers:

```toml title="pyproject.toml"
[project]
dependencies = ["torch"]

[tool.uv.sources]
torch = [
  { index = "pytorch-cpu", marker = "sys_platform == 'darwin'"},
  { index = "pytorch-cu130", marker = "sys_platform != 'darwin'"},
]

[[tool.uv.index]]
name = "pytorch-cpu"
url = "https://download.pytorch.org/whl/cpu"

[[tool.uv.index]]
name = "pytorch-cu130"
url = "https://download.pytorch.org/whl/cu130"
```

Set `explicit = true` to restrict an index to packages explicitly pinned to it. In the following
`pyproject.toml` example, `torch` comes from the `pytorch` index, and all other packages come from
PyPI:

```toml
[tool.uv.sources]
torch = { index = "pytorch" }

[[tool.uv.index]]
name = "pytorch"
url = "https://download.pytorch.org/whl/cpu"
explicit = true
```

Define any named index that `tool.uv.sources` references in the project `pyproject.toml` file. uv
does not recognize indexes from the command line, environment variables, or user-level configuration
for these references.

If an index sets both `default = true` and `explicit = true`, uv uses it only through
`tool.uv.sources`. This index also removes PyPI as the default index.

## Searching across multiple indexes

By default, uv stops at the first index that contains a package. It considers only versions on that
index. This strategy is `first-index`.

For example, if an internal `[[tool.uv.index]]` contains a package, uv _always_ installs that
package from the internal index. It never installs that package from PyPI. This behavior prevents
"dependency confusion" attacks. In these attacks, a malicious PyPI package uses the same name as an
internal package. See
[the `torchtriton` attack](https://pytorch.org/blog/compromised-nightly-dependency/) from
December 2022.

To select another index strategy, set `--index-strategy` or `UV_INDEX_STRATEGY` to one of these
values:

- `first-index` (default): Search each index. Consider only versions on the first index that
  contains the package.
- `unsafe-first-match`: Search each index. Prefer the first index with a compatible version, even if
  other indexes contain newer versions.
- `unsafe-best-match`: Search all indexes and select the best version from all available versions.

The `unsafe-best-match` strategy most closely matches pip behavior. However, it exposes users to
"dependency confusion" attacks.

## Authentication

Most private package indexes require a username and password or an access token.

!!! tip

    These guides explain authentication for specific private index providers:
    [Azure Artifacts](../guides/integration/azure.md),
    [Google Artifact Registry](../guides/integration/google.md),
    [AWS CodeArtifact](../guides/integration/aws.md), and
    [JFrog Artifactory](../guides/integration/jfrog.md).

### Providing credentials directly

Provide credentials in environment variables or directly in the index URL.

For example, an index named `internal-proxy` requires the username `public` and password `koala`.
Define the index without credentials in `pyproject.toml`:

```toml
[[tool.uv.index]]
name = "internal-proxy"
url = "https://example.com/simple"
```

Then set `UV_INDEX_INTERNAL_PROXY_USERNAME` and `UV_INDEX_INTERNAL_PROXY_PASSWORD`. In these
variable names, `INTERNAL_PROXY` is the uppercase index name. Underscores replace non-alphanumeric
characters:

```sh
export UV_INDEX_INTERNAL_PROXY_USERNAME=public
export UV_INDEX_INTERNAL_PROXY_PASSWORD=koala
```

Environment variables keep sensitive credentials out of the plaintext `pyproject.toml` file.

Alternatively, include credentials directly in the index definition:

```toml
[[tool.uv.index]]
name = "internal"
url = "https://public:koala@pypi-proxy.corp.dev/simple"
```

uv _never_ stores credentials in `uv.lock`. It _must_ have access to the authenticated URL when it
installs packages.

### Using credential providers

uv can also find credentials in netrc and keyring. See the
[HTTP authentication](./authentication/http.md) documentation to configure specific credential
providers.

By default, uv first sends a request without credentials. If the request fails, uv searches for
credentials. If it finds credentials, uv sends an authenticated request.

!!! note

    If a username is set, uv searches for credentials before it sends an unauthenticated request.

Some indexes, such as GitLab, forward unauthenticated requests to public indexes such as PyPI. When
this happens, uv does not search for credentials. Set `authenticate` for an index to change this
behavior. For example, always search for credentials:

```toml hl_lines="4"
[[tool.uv.index]]
name = "example"
url = "https://example.com/simple"
authenticate = "always"
```

If `authenticate` is `always`, uv searches for credentials immediately and fails if it cannot find
them.

### Ignoring error codes

With the [first-index strategy](#searching-across-multiple-indexes), uv stops if an index returns
HTTP 401 Unauthorized or HTTP 403 Forbidden. The `pytorch` index is an exception. uv ignores HTTP
403 responses from this index because it returns that status when a package does not exist.

By default, uv also stops resolution if an HTTP error occurs while it retrieves distribution
metadata or an archive. If uv ignores the error, the affected version becomes unavailable and the
resolver can try another version.

Use `ignore-error-codes` to select which errors uv ignores for an index. For example, ignore HTTP
403 but not HTTP 401 for a private index:

```toml
[[tool.uv.index]]
name = "private-index"
url = "https://private-index.com/simple"
authenticate = "always"
ignore-error-codes = [403]
```

If an index returns `404 Not Found`, uv always searches the next index. This behavior cannot change.

### Disabling authentication

To prevent credential leaks, disable authentication for an index:

```toml hl_lines="4"
[[tool.uv.index]]
name = "example"
url = "https://example.com/simple"
authenticate = "never"
```

If `authenticate` is `never`, uv does not search for credentials for that index. It fails if the
user provides credentials directly.

### Customizing cache control headers

By default, uv follows the cache control headers from each index. PyPI serves package metadata with
`max-age=600`, so uv caches metadata for 10 minutes. PyPI serves wheels and source distributions
with `max-age=365000000, immutable`, so uv can cache those artifacts indefinitely.

To override the cache control headers for an index, use the `cache-control` setting:

```toml
[[tool.uv.index]]
name = "example"
url = "https://example.com/simple"
cache-control = { api = "max-age=600", files = "max-age=365000000, immutable" }
```

The `cache-control` setting accepts an object with two optional keys:

- `api`: Controls caching for Simple API requests (package metadata).
- `files`: Controls caching for artifact downloads (wheels and source distributions).

The values use
[HTTP Cache-Control](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Cache-Control)
syntax. To make uv always revalidate package metadata, set `api = "no-cache"`:

```toml
[[tool.uv.index]]
name = "example"
url = "https://example.com/simple"
cache-control = { api = "no-cache" }
```

This setting can override private indexes that unintentionally disable caching. The recommended
values match PyPI: `api = "max-age=600"` and `files = "max-age=365000000, immutable"`.

### Requiring a hash algorithm

If an index advertises multiple hashes for a distribution, uv records one hash in the lockfile. To
require a specific algorithm for an index, use `hash-algorithm`:

```toml
[tool.uv]
preview-features = ["index-hash-algorithm"]

[[tool.uv.index]]
name = "private-index"
url = "https://private-index.com/simple"
hash-algorithm = "sha256"
```

If a locked distribution does not advertise the required algorithm, uv fails. It does not use
another hash algorithm.

### Configuring `exclude-newer` for an index

If [`exclude-newer`](./resolution.md#reproducible-resolutions) is configured, an index can specify a
different cutoff:

```toml
[[tool.uv.index]]
name = "internal"
url = "https://internal.example.com/simple"
exclude-newer = "7 days"
```

An index-specific cutoff affects only packages from that index. Package-specific
`exclude-newer-package` settings still take precedence.

If an index does not provide `upload-time` metadata, disable the cutoff for that index:

```toml
[[tool.uv.index]]
name = "internal"
url = "https://internal.example.com/simple"
exclude-newer = false
```

## "Flat" indexes

By default, uv treats `[[tool.uv.index]]` entries as PyPI-style registries that implement the
[PEP 503](https://peps.python.org/pep-0503/) Simple Repository API. uv also supports "flat" indexes.
These indexes are local directories or HTML pages that list wheels and source distributions. In pip,
the `--find-links` option specifies these indexes.

The `format = "flat"` option defines a flat index in `pyproject.toml`:

```toml
[[tool.uv.index]]
name = "example"
url = "/path/to/directory"
format = "flat"
```

Flat indexes support the same features as Simple Repository API indexes, including
`explicit = true`. The `tool.uv.sources` setting can also pin a package to a flat index.

## `--index-url` and `--extra-index-url`

uv also supports the pip-style `--index-url` and `--extra-index-url` options for compatibility. The
`--index-url` option defines the default index. The `--extra-index-url` option defines additional
indexes.

These options work with `[[tool.uv.index]]` and follow the same priority rules:

- The default index always has the lowest priority. Define it with legacy `--index-url`, recommended
  `--default-index`, or a `[[tool.uv.index]]` entry with `default = true`.
- uv searches indexes in their defined order. Define them with legacy `--extra-index-url`,
  recommended `--index`, or `[[tool.uv.index]]` entries.

The `--index-url` and `--extra-index-url` options behave like unnamed `[[tool.uv.index]]` entries.
The `--index-url` entry also sets `default = true`. Thus, `--index-url` corresponds to
`--default-index`, and `--extra-index-url` corresponds to `--index`.
